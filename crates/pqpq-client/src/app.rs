//! Screen state, server messages, and the 30 Hz input/prediction clock.

use std::time::{Duration, Instant};

use pqpq_protocol::{
    CarSnapshot, CarStatus, ClientMessage, Controls, ErrorCode, GameSnapshot, InputDatagram, Join,
    PROTOCOL_VERSION, PlayerId, RacePhase, Role, RoomView, ServerMessage,
};
use pqpq_sim::{
    COURSE_ID, COURSE_VERSION, CarState, Course, FIXED_DT_SECONDS, InputState, Interpolator, LAPS,
    PHYSICS_VERSION, Predictor, Steering, TICK_HZ, TickClock,
};
use tokio::sync::mpsc::UnboundedSender;

use crate::config::Config;
use crate::input::{Command, Keys};
use crate::network::{NetCommand, NetEvent};

/// Other cars are drawn this far behind the newest snapshot.
const INTERPOLATION_DELAY_TICKS: f64 = 3.0;
const PING_INTERVAL: Duration = Duration::from_secs(1);
const NOTICE_TIME: Duration = Duration::from_secs(3);
/// No snapshot for this long during a race shows a delay warning.
const STALE_AFTER: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Screen {
    Connecting,
    Lobby,
    Countdown,
    Race,
    Spectate,
    Results,
    Error,
}

pub struct CarView {
    pub id: PlayerId,
    pub label: String,
    pub state: CarState,
    pub me: bool,
}

pub struct App {
    pub cfg: Config,
    pub course: Course,
    pub view: Option<RoomView>,
    pub error: Option<String>,
    pub notice: Option<(String, Instant)>,
    pub keys: Keys,
    /// Terminal too small or unfocused: controls are forced neutral.
    pub input_blocked: bool,
    pub last_snapshot: Option<GameSnapshot>,
    last_snapshot_at: Option<Instant>,
    pub rtt: Option<Duration>,
    pub quit: bool,
    predictor: Predictor,
    interp: Interpolator,
    clock: TickClock,
    seq: u64,
    acc: f64,
    ping: Option<(u64, Instant)>,
    next_ping: Instant,
    epoch: Instant,
    net: UnboundedSender<NetCommand>,
}

impl App {
    pub fn new(cfg: Config, keys: Keys, net: UnboundedSender<NetCommand>) -> App {
        let now = Instant::now();
        App {
            cfg,
            course: Course::standard(),
            view: None,
            error: None,
            notice: None,
            keys,
            input_blocked: false,
            last_snapshot: None,
            last_snapshot_at: None,
            rtt: None,
            quit: false,
            predictor: Predictor::default(),
            interp: Interpolator::default(),
            clock: TickClock::default(),
            seq: 0,
            acc: 0.0,
            ping: None,
            next_ping: now,
            epoch: now,
            net,
        }
    }

    pub fn screen(&self) -> Screen {
        if self.error.is_some() {
            return Screen::Error;
        }
        let Some(view) = &self.view else {
            return Screen::Connecting;
        };
        match view.phase {
            RacePhase::Waiting if view.start.is_some() => Screen::Countdown,
            RacePhase::Waiting => Screen::Lobby,
            RacePhase::Racing if self.is_entrant() => Screen::Race,
            RacePhase::Racing => Screen::Spectate,
            RacePhase::Finished => Screen::Results,
        }
    }

    fn is_entrant(&self) -> bool {
        self.view.as_ref().is_some_and(|v| {
            v.role == Role::Racer
                && v.start
                    .as_ref()
                    .is_some_and(|s| s.grid.iter().any(|g| g.player_id == v.me))
        })
    }

    fn send(&self, msg: ClientMessage) {
        let _ = self.net.send(NetCommand::Send(msg));
    }

    fn fail(&mut self, message: String) {
        if self.error.is_none() {
            self.error = Some(message);
        }
        let _ = self.net.send(NetCommand::Close);
    }

    pub fn on_command(&mut self, command: Command) {
        match command {
            Command::Quit => {
                if self.view.is_some() {
                    self.send(ClientMessage::Leave);
                }
                let _ = self.net.send(NetCommand::Close);
                self.quit = true;
            },
            Command::Ready if self.screen() == Screen::Lobby => self.send(ClientMessage::Ready),
            Command::Ready => {},
        }
    }

    pub fn on_net(&mut self, event: NetEvent, now: Instant) {
        match event {
            NetEvent::Connected => self.send(ClientMessage::Join(Join {
                protocol_version: PROTOCOL_VERSION,
                username: self.cfg.username.clone(),
                room_id: self.cfg.room_id.clone(),
                course_id: COURSE_ID,
                course_version: COURSE_VERSION,
                physics_version: PHYSICS_VERSION,
            })),
            NetEvent::Closed(reason) => {
                if !self.quit && self.error.is_none() {
                    self.error = Some(format!(
                        "{reason}。再起動すると新しいプレイヤーとして参加できます"
                    ));
                }
            },
            NetEvent::Snapshot(snap) => {
                if let Some(snap) = self.view.as_mut().and_then(|v| v.offer(snap)) {
                    self.apply_snapshot(snap, now);
                }
            },
            NetEvent::Message(msg) => self.on_message(msg, now),
        }
    }

    fn on_message(&mut self, msg: ServerMessage, now: Instant) {
        let now_s = self.seconds(now);
        match msg {
            ServerMessage::Joined(j) => {
                if self.view.is_some() {
                    return self.fail("通信形式が一致しません".into());
                }
                if (
                    j.course_id,
                    j.course_version,
                    j.physics_version,
                    j.tick_hz as u32,
                    j.laps as u32,
                ) != (COURSE_ID, COURSE_VERSION, PHYSICS_VERSION, TICK_HZ, LAPS)
                {
                    return self.fail(ErrorCode::VersionMismatch.message().into());
                }
                self.clock.observe(j.server_tick, now_s);
                self.view = Some(RoomView::new(j));
            },
            ServerMessage::Error {
                message,
                fatal: true,
                ..
            } => self.fail(message),
            ServerMessage::Error { message, .. } => self.notice = Some((message, now)),
            ServerMessage::Ping { nonce } => self.send(ClientMessage::Pong { nonce }),
            ServerMessage::Pong { nonce } => {
                if let Some((n, sent)) = self.ping
                    && n == nonce
                {
                    self.rtt = Some(now - sent);
                    self.ping = None;
                }
            },
            msg => {
                if let ServerMessage::RaceStart { start, .. } = &msg {
                    self.clock.observe(start.server_tick, now_s);
                }
                let Some(view) = self.view.as_mut() else {
                    return self.fail("通信形式が一致しません".into());
                };
                for snap in view.apply(&msg) {
                    self.apply_snapshot(snap, now);
                }
                if matches!(msg, ServerMessage::RaceFinished { .. }) {
                    self.predictor.stop();
                }
            },
        }
    }

    fn apply_snapshot(&mut self, snap: GameSnapshot, now: Instant) {
        self.clock.observe(snap.tick, self.seconds(now));
        let me = self.view.as_ref().map(|v| v.me);
        let cars: Vec<(u64, CarState)> = snap
            .cars
            .iter()
            .map(|c| (c.player_id.0, to_state(c)))
            .collect();
        self.interp.push(snap.tick, cars);
        match snap.cars.iter().find(|c| Some(c.player_id) == me) {
            Some(c) if c.status == CarStatus::Racing => {
                self.predictor
                    .reconcile(&self.course, to_state(c), snap.last_processed_input);
            },
            _ => self.predictor.stop(),
        }
        self.last_snapshot = Some(snap);
        self.last_snapshot_at = Some(now);
    }

    /// Called every rendered frame.
    pub fn frame(&mut self, now: Instant, dt: f64) {
        if let Some((_, at)) = &self.notice
            && now - *at > NOTICE_TIME
        {
            self.notice = None;
        }
        if self.error.is_some() || self.quit {
            return;
        }
        if self.view.is_some() && now >= self.next_ping {
            let nonce = self.next_ping.duration_since(self.epoch).as_nanos() as u64;
            self.ping = Some((nonce, now));
            self.send(ClientMessage::Ping { nonce });
            self.next_ping = now + PING_INTERVAL;
        }
        self.predictor.decay(dt);
        let sending = self
            .view
            .as_ref()
            .is_some_and(|v| v.start.is_some() && v.phase != RacePhase::Finished)
            && self.is_entrant();
        self.acc += dt;
        let mut steps = 0;
        while self.acc >= FIXED_DT_SECONDS && steps < 3 {
            self.acc -= FIXED_DT_SECONDS;
            steps += 1;
            if !sending {
                continue;
            }
            let input = if self.input_blocked {
                InputState::default()
            } else {
                self.keys.state(now)
            };
            self.seq += 1;
            let _ = self.net.send(NetCommand::Input(InputDatagram {
                sequence: self.seq,
                controls: to_controls(input),
            }));
            self.predictor.step(&self.course, self.seq, input);
        }
        // After a stall, resynchronize instead of replaying old input.
        if self.acc >= FIXED_DT_SECONDS {
            self.acc = 0.0;
        }
    }

    fn seconds(&self, now: Instant) -> f64 {
        now.duration_since(self.epoch).as_secs_f64()
    }

    /// Estimated current server tick, including half the round trip.
    fn server_tick(&self, now: Instant) -> Option<f64> {
        let half_rtt = self.rtt.map_or(0.0, |r| r.as_secs_f64() / 2.0);
        self.clock
            .tick_at(self.seconds(now))
            .map(|t| t + half_rtt * TICK_HZ as f64)
    }

    pub fn countdown(&self, now: Instant) -> Option<f64> {
        let start = self.view.as_ref()?.start.as_ref()?;
        Some(((start.start_tick as f64 - self.server_tick(now)?) / TICK_HZ as f64).max(0.0))
    }

    pub fn race_time(&self, now: Instant) -> Option<f64> {
        let view = self.view.as_ref()?;
        let start = view.start.as_ref()?;
        if view.phase != RacePhase::Racing {
            return None;
        }
        Some(((self.server_tick(now)? - start.start_tick as f64) / TICK_HZ as f64).max(0.0))
    }

    pub fn stale(&self, now: Instant) -> bool {
        self.screen() == Screen::Race
            && self.last_snapshot_at.is_some_and(|t| now - t > STALE_AFTER)
    }

    pub fn my_snapshot(&self) -> Option<&CarSnapshot> {
        let me = self.view.as_ref()?.me;
        self.last_snapshot
            .as_ref()?
            .cars
            .iter()
            .find(|c| c.player_id == me)
    }

    /// Cars to draw: own car predicted, others interpolated, grid before start.
    pub fn cars(&self, now: Instant) -> Vec<CarView> {
        let Some(view) = &self.view else {
            return Vec::new();
        };
        let label = |id: PlayerId| view.label_of(id);
        if self.interp.latest_tick().is_none() {
            let Some(start) = &view.start else {
                return Vec::new();
            };
            return start
                .grid
                .iter()
                .filter(|g| view.phase == RacePhase::Waiting || view.name_of(g.player_id).is_some())
                .map(|g| CarView {
                    id: g.player_id,
                    label: label(g.player_id),
                    state: CarState {
                        position: [g.position[0] as f64, g.position[1] as f64],
                        direction: g.direction as f64,
                        ..Default::default()
                    },
                    me: g.player_id == view.me,
                })
                .collect();
        }
        let render_tick =
            self.clock.tick_at(self.seconds(now)).unwrap_or(0.0) - INTERPOLATION_DELAY_TICKS;
        let mut cars: Vec<CarView> = self
            .interp
            .sample(render_tick)
            .into_iter()
            .filter(|(id, _)| !(self.predictor.is_active() && *id == view.me.0))
            .map(|(id, state)| CarView {
                id: PlayerId(id),
                label: label(PlayerId(id)),
                state,
                me: id == view.me.0,
            })
            .collect();
        if self.predictor.is_active() {
            let alpha = (self.acc / FIXED_DT_SECONDS).clamp(0.0, 1.0);
            cars.push(CarView {
                id: view.me,
                label: label(view.me),
                state: self.predictor.render(alpha),
                me: true,
            });
        }
        cars
    }
}

fn to_state(c: &CarSnapshot) -> CarState {
    CarState {
        position: [c.position[0] as f64, c.position[1] as f64],
        velocity: [c.velocity[0] as f64, c.velocity[1] as f64],
        direction: c.direction as f64,
        laps: c.laps as u32,
        next_checkpoint: c.next_checkpoint as u32,
    }
}

fn to_controls(i: InputState) -> Controls {
    let steering = match i.steering {
        Steering::Left => 1,
        Steering::Neutral => 0,
        Steering::Right => -1,
    };
    Controls {
        throttle: i.throttle,
        brake: i.brake,
        steering,
    }
}
