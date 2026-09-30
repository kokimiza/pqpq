//! Opt-in real-network acceptance driver, not an in-game NPC.
//! Uses the production native transport and only sends legal controls.
#[path = "../src/config.rs"]
mod config;
#[path = "../src/network.rs"]
mod network;

use network::{NetCommand, NetEvent};
use pqpq_protocol::{
    CarStatus, ClientMessage, Controls, InputDatagram, Join, PROTOCOL_VERSION, ServerMessage,
};
use pqpq_sim::{COURSE_ID, COURSE_VERSION, Course, PHYSICS_VERSION};
use std::time::Duration;
use tokio::sync::mpsc;

#[tokio::test]
#[ignore = "requires a running test server and a second racer; run via npm run test:crossplay"]
async fn native_three_laps() {
    let args = vec![
        "same-name".to_owned(),
        std::env::var("PQPQ_TEST_ROOM").unwrap(),
    ];
    let cfg = config::Config::from_args(&args).unwrap_or_else(|error| match error {
        config::ArgsError::Invalid(message) => panic!("test client configuration: {message}"),
        config::ArgsError::Usage => panic!("test client argument count"),
    });
    let (tx, rx) = mpsc::unbounded_channel();
    let (events, mut incoming) = mpsc::unbounded_channel();
    let network = tokio::spawn(network::run(cfg.clone(), rx, events));
    let course = Course::standard();
    let mut me = None;
    let mut seq = 0;
    let mut controls = Controls {
        throttle: false,
        brake: false,
        steering: 0,
    };
    let mut racing = false;
    let mut clock = tokio::time::interval(Duration::from_secs_f64(1.0 / 30.0));
    clock.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let deadline = tokio::time::sleep(Duration::from_secs(120));
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            _ = &mut deadline => panic!("native race timed out"),
            _ = clock.tick(), if racing => {
                seq += 1;
                tx.send(NetCommand::Input(InputDatagram { sequence: seq, controls })).unwrap();
            }
            event = incoming.recv() => match event.expect("network event channel") {
                NetEvent::Connected => tx.send(NetCommand::Send(ClientMessage::Join(Join {
                    protocol_version: PROTOCOL_VERSION, username: cfg.username.clone(), room_id: cfg.room_id.clone(),
                    course_id: COURSE_ID, course_version: COURSE_VERSION, physics_version: PHYSICS_VERSION,
                }))).unwrap(),
                NetEvent::Message(ServerMessage::Joined(joined)) => {
                    me = Some(joined.player_id);
                    println!("NATIVE_JOINED {}", joined.player_id);
                    tx.send(NetCommand::Send(ClientMessage::Ready)).unwrap();
                }
                NetEvent::Message(ServerMessage::Ping { nonce }) => {
                    tx.send(NetCommand::Send(ClientMessage::Pong { nonce })).unwrap();
                }
                NetEvent::Snapshot(s) => {
                    if let Some(car) = s.cars.iter().find(|c| Some(c.player_id) == me) {
                        racing = car.status == CarStatus::Racing;
                        let position = [car.position[0] as f64, car.position[1] as f64];
                        let projection = course.project(position);
                        let target = course.point_at(projection.s + 15.0).0;
                        let desired = (target[1] - position[1]).atan2(target[0] - position[0]);
                        let diff = (desired - car.direction as f64 + std::f64::consts::PI)
                            .rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI;
                        controls = Controls {
                            throttle: (car.velocity[0] as f64).hypot(car.velocity[1] as f64) < 30.0,
                            brake: false,
                            steering: if diff > 0.05 { 1 } else if diff < -0.05 { -1 } else { 0 },
                        };
                    }
                }
                NetEvent::Message(ServerMessage::RaceFinished { results, .. }) => {
                    assert_eq!(results.len(), 2);
                    assert!(results.iter().all(|r| r.status == CarStatus::Finished && r.laps == 3), "{results:?}");
                    println!("NATIVE_RESULTS {}", results.iter().map(|r| format!("{}:{}", r.player_id.0, r.finish_time_ms.unwrap())).collect::<Vec<_>>().join(","));
                    tx.send(NetCommand::Send(ClientMessage::Leave)).unwrap();
                    tx.send(NetCommand::Close).unwrap();
                    break;
                }
                NetEvent::Message(ServerMessage::Error { message, .. }) => panic!("server error: {message}"),
                NetEvent::Closed(reason) => panic!("native disconnected: {reason}"),
                _ => {}
            }
        }
    }
    tokio::time::timeout(Duration::from_secs(1), network)
        .await
        .unwrap()
        .unwrap();
}
