//! Browser session without any Web API: codec, room state, prediction,
//! reconciliation and interpolation. JS feeds bytes and time in, takes bytes out.

use pqpq_protocol::{
    CarSnapshot, CarStatus, ClientMessage, Controls, ErrorCode, FrameDecoder, GameSnapshot,
    InputDatagram, Join, PROTOCOL_VERSION, PlayerId, RacePhase, Role, RoomView, ServerMessage,
    SnapshotAssembler, SnapshotFragment, validate_room_id, validate_username,
};
use pqpq_sim::{
    COURSE_ID, COURSE_VERSION, CarState, Course, FIXED_DT_SECONDS, InputState, Interpolator, LAPS,
    PHYSICS_VERSION, Predictor, Steering, TICK_HZ, TickClock,
};

const INTERPOLATION_DELAY_TICKS: f64 = 3.0;
const PING_INTERVAL_MS: f64 = 1000.0;
const NOTICE_MS: f64 = 3000.0;
const STALE_AFTER_MS: f64 = 250.0;

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

impl Screen {
    pub fn name(self) -> &'static str {
        match self {
            Screen::Connecting => "connecting",
            Screen::Lobby => "lobby",
            Screen::Countdown => "countdown",
            Screen::Race => "race",
            Screen::Spectate => "spectate",
            Screen::Results => "results",
            Screen::Error => "error",
        }
    }
}

pub struct CarView {
    pub id: PlayerId,
    pub state: CarState,
    pub me: bool,
}

#[derive(Default)]
pub struct Session {
    pub course: Course,
    pub view: Option<RoomView>,
    pub error: Option<String>,
    notice: Option<(String, f64)>,
    decoder: FrameDecoder,
    assembler: SnapshotAssembler,
    predictor: Predictor,
    interp: Interpolator,
    clock: TickClock,
    pub last_snapshot: Option<GameSnapshot>,
    last_snapshot_ms: Option<f64>,
    input: InputState,
    seq: u64,
    acc: f64,
    last_ms: Option<f64>,
    pub rtt_ms: Option<f64>,
    ping: Option<(u64, f64)>,
    next_ping_ms: f64,
    out_stream: Vec<u8>,
    out_datagrams: Vec<Vec<u8>>,
}

impl Session {
    pub fn new() -> Session {
        Session::default()
    }

    /// Validates like the server and queues Join as the first message.
    pub fn join(&mut self, username: &str, room_id: &str) -> Result<(), String> {
        validate_username(username).map_err(|e| e.to_string())?;
        validate_room_id(room_id).map_err(|e| e.to_string())?;
        self.send(ClientMessage::Join(Join {
            protocol_version: PROTOCOL_VERSION,
            username: username.to_owned(),
            room_id: room_id.to_owned(),
            course_id: COURSE_ID,
            course_version: COURSE_VERSION,
            physics_version: PHYSICS_VERSION,
        }));
        Ok(())
    }

    pub fn ready(&mut self) {
        if self.screen() == Screen::Lobby {
            self.send(ClientMessage::Ready);
        }
    }

    pub fn leave(&mut self) {
        self.send(ClientMessage::Leave);
    }

    fn send(&mut self, msg: ClientMessage) {
        self.out_stream.extend_from_slice(&msg.encode_frame());
    }

    pub fn take_stream(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.out_stream)
    }

    pub fn take_datagram(&mut self) -> Option<Vec<u8>> {
        (!self.out_datagrams.is_empty()).then(|| self.out_datagrams.remove(0))
    }

    pub fn fail(&mut self, message: String) {
        if self.error.is_none() {
            self.error = Some(message);
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

    /// Control stream bytes; may hold partial or several messages.
    pub fn on_stream(&mut self, bytes: &[u8], now_ms: f64) -> Result<(), String> {
        self.decoder.push(bytes);
        loop {
            let body = match self.decoder.next_frame() {
                Ok(Some(body)) => body,
                Ok(None) => return Ok(()),
                Err(_) => break,
            };
            match ServerMessage::decode(&body) {
                Ok(msg) => self.on_message(msg, now_ms),
                Err(_) => break,
            }
        }
        let msg = ErrorCode::ProtocolViolation.message().to_owned();
        self.fail(msg.clone());
        Err(msg)
    }

    fn on_message(&mut self, msg: ServerMessage, now_ms: f64) {
        let now_s = now_ms / 1000.0;
        match msg {
            ServerMessage::Joined(j) => {
                if self.view.is_some() {
                    return self.fail(ErrorCode::ProtocolViolation.message().into());
                }
                if (
                    j.course_id,
                    j.course_version,
                    j.physics_version,
                    j.tick_hz as u32,
                    j.laps as u32,
                ) != (COURSE_ID, COURSE_VERSION, PHYSICS_VERSION, TICK_HZ, LAPS)
                {
                    return self.fail(
                        "対応するクライアントが必要です。ページを再読み込みしてください".into(),
                    );
                }
                self.clock.observe(j.server_tick, now_s);
                self.view = Some(RoomView::new(j));
            },
            ServerMessage::Error {
                message,
                fatal: true,
                ..
            } => self.fail(message),
            ServerMessage::Error { message, .. } => self.notice = Some((message, now_ms)),
            ServerMessage::Ping { nonce } => self.send(ClientMessage::Pong { nonce }),
            ServerMessage::Pong { nonce } => {
                if let Some((n, sent)) = self.ping
                    && n == nonce
                {
                    self.rtt_ms = Some(now_ms - sent);
                    self.ping = None;
                }
            },
            msg => {
                if let ServerMessage::RaceStart { start, .. } = &msg {
                    self.clock.observe(start.server_tick, now_s);
                }
                let Some(view) = self.view.as_mut() else {
                    return self.fail(ErrorCode::ProtocolViolation.message().into());
                };
                for snap in view.apply(&msg) {
                    self.apply_snapshot(snap, now_ms);
                }
                if matches!(msg, ServerMessage::RaceFinished { .. }) {
                    self.predictor.stop();
                }
            },
        }
    }

    pub fn on_datagram(&mut self, bytes: &[u8], now_ms: f64) {
        let Ok(fragment) = SnapshotFragment::decode(bytes) else {
            return;
        };
        let Some(body) = self.assembler.push(fragment, now_ms) else {
            return;
        };
        let Ok(snap) = GameSnapshot::decode(&body) else {
            return;
        };
        if let Some(snap) = self.view.as_mut().and_then(|v| v.offer(snap)) {
            self.apply_snapshot(snap, now_ms);
        }
    }

    fn apply_snapshot(&mut self, snap: GameSnapshot, now_ms: f64) {
        self.clock.observe(snap.tick, now_ms / 1000.0);
        let me = self.view.as_ref().map(|v| v.me);
        self.interp.push(
            snap.tick,
            snap.cars
                .iter()
                .map(|c| (c.player_id.0, to_state(c)))
                .collect(),
        );
        match snap.cars.iter().find(|c| Some(c.player_id) == me) {
            Some(c) if c.status == CarStatus::Racing => {
                self.predictor
                    .reconcile(&self.course, to_state(c), snap.last_processed_input);
            },
            _ => self.predictor.stop(),
        }
        self.last_snapshot = Some(snap);
        self.last_snapshot_ms = Some(now_ms);
    }

    pub fn set_keys(&mut self, up: bool, down: bool, left: bool, right: bool) {
        self.input = InputState::from_keys(up, down, left, right);
    }

    /// Focus lost or page hidden: neutral controls, sent right away.
    pub fn neutral(&mut self) {
        self.input = InputState::default();
        if self.sending() {
            self.seq += 1;
            self.queue_input(self.input);
        }
    }

    fn sending(&self) -> bool {
        self.error.is_none()
            && self
                .view
                .as_ref()
                .is_some_and(|v| v.start.is_some() && v.phase != RacePhase::Finished)
            && self.is_entrant()
    }

    fn queue_input(&mut self, input: InputState) {
        let datagram = InputDatagram {
            sequence: self.seq,
            controls: to_controls(input),
        }
        .encode();
        // Only the newest few matter; never let a stalled page pile them up.
        if self.out_datagrams.len() >= 4 {
            self.out_datagrams.remove(0);
        }
        self.out_datagrams.push(datagram);
    }

    /// Advances the fixed 30 Hz input/prediction clock; call every frame.
    pub fn update(&mut self, now_ms: f64) {
        let dt = self
            .last_ms
            .map_or(0.0, |last| ((now_ms - last) / 1000.0).max(0.0));
        self.last_ms = Some(now_ms);
        if self
            .notice
            .as_ref()
            .is_some_and(|(_, at)| now_ms - at > NOTICE_MS)
        {
            self.notice = None;
        }
        if self.error.is_some() {
            return;
        }
        if self.view.is_some() && now_ms >= self.next_ping_ms {
            let nonce = now_ms as u64;
            self.ping = Some((nonce, now_ms));
            self.send(ClientMessage::Ping { nonce });
            self.next_ping_ms = now_ms + PING_INTERVAL_MS;
        }
        self.predictor.decay(dt);
        let sending = self.sending();
        self.acc += dt;
        let mut steps = 0;
        while self.acc >= FIXED_DT_SECONDS && steps < 3 {
            self.acc -= FIXED_DT_SECONDS;
            steps += 1;
            if sending {
                self.seq += 1;
                self.queue_input(self.input);
                self.predictor.step(&self.course, self.seq, self.input);
            }
        }
        // A throttled or frozen tab resynchronizes instead of replaying.
        if self.acc >= FIXED_DT_SECONDS {
            self.acc = 0.0;
        }
    }

    pub fn notice(&self) -> Option<&str> {
        self.notice.as_ref().map(|(m, _)| m.as_str())
    }

    fn server_tick(&self, now_ms: f64) -> Option<f64> {
        let half_rtt = self.rtt_ms.unwrap_or(0.0) / 2000.0;
        self.clock
            .tick_at(now_ms / 1000.0)
            .map(|t| t + half_rtt * TICK_HZ as f64)
    }

    pub fn countdown(&self, now_ms: f64) -> Option<f64> {
        let start = self.view.as_ref()?.start.as_ref()?;
        Some(((start.start_tick as f64 - self.server_tick(now_ms)?) / TICK_HZ as f64).max(0.0))
    }

    pub fn race_time(&self, now_ms: f64) -> Option<f64> {
        let view = self.view.as_ref()?;
        if view.phase != RacePhase::Racing {
            return None;
        }
        let start = view.start.as_ref()?;
        Some(((self.server_tick(now_ms)? - start.start_tick as f64) / TICK_HZ as f64).max(0.0))
    }

    pub fn stale(&self, now_ms: f64) -> bool {
        self.screen() == Screen::Race
            && self
                .last_snapshot_ms
                .is_some_and(|t| now_ms - t > STALE_AFTER_MS)
    }

    pub fn my_snapshot(&self) -> Option<&CarSnapshot> {
        let me = self.view.as_ref()?.me;
        self.last_snapshot
            .as_ref()?
            .cars
            .iter()
            .find(|c| c.player_id == me)
    }

    /// Own car predicted, others interpolated, grid before the first snapshot.
    pub fn cars(&self, now_ms: f64) -> Vec<CarView> {
        let Some(view) = &self.view else {
            return Vec::new();
        };
        if self.interp.latest_tick().is_none() {
            let Some(start) = &view.start else {
                return Vec::new();
            };
            return start
                .grid
                .iter()
                .map(|g| CarView {
                    id: g.player_id,
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
            self.clock.tick_at(now_ms / 1000.0).unwrap_or(0.0) - INTERPOLATION_DELAY_TICKS;
        let predicting = self.predictor.is_active();
        let mut cars: Vec<CarView> = self
            .interp
            .sample(render_tick)
            .into_iter()
            .filter(|(id, _)| !(predicting && *id == view.me.0))
            .map(|(id, state)| CarView {
                id: PlayerId(id),
                state,
                me: id == view.me.0,
            })
            .collect();
        if predicting {
            let alpha = (self.acc / FIXED_DT_SECONDS).clamp(0.0, 1.0);
            cars.push(CarView {
                id: view.me,
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

#[cfg(test)]
mod tests {
    use super::*;
    use pqpq_protocol::{Joined, RosterEntry};

    fn joined() -> ServerMessage {
        ServerMessage::Joined(Joined {
            player_id: PlayerId(7),
            room_id: "1234".into(),
            role: Role::Racer,
            roster: vec![RosterEntry {
                player_id: PlayerId(7),
                username: "foo".into(),
                role: Role::Racer,
                ready: false,
            }],
            room_revision: 1,
            phase: RacePhase::Waiting,
            server_tick: 100,
            start: None,
            course_id: COURSE_ID,
            course_version: COURSE_VERSION,
            physics_version: PHYSICS_VERSION,
            tick_hz: TICK_HZ as u16,
            laps: LAPS as u8,
            min_racers: 2,
            results: vec![],
        })
    }

    #[test]
    fn join_validates_and_queues_one_frame() {
        let mut s = Session::new();
        assert!(s.join("\n", "1234").is_err());
        assert!(s.take_stream().is_empty());
        s.join("foo", "1234").unwrap();
        let mut d = FrameDecoder::default();
        d.push(&s.take_stream());
        assert!(matches!(
            ClientMessage::decode(&d.next_frame().unwrap().unwrap()),
            Ok(ClientMessage::Join(_))
        ));
    }

    #[test]
    fn split_stream_bytes_reach_the_lobby() {
        let mut s = Session::new();
        let frame = joined().encode_frame();
        let (a, b) = frame.split_at(5);
        s.on_stream(a, 0.0).unwrap();
        assert_eq!(s.screen(), Screen::Connecting);
        s.on_stream(b, 0.0).unwrap();
        assert_eq!(s.screen(), Screen::Lobby);
        // Lobby: nothing is sent at 30 Hz yet.
        s.update(0.0);
        s.update(1000.0);
        assert!(s.take_datagram().is_none());
    }

    #[test]
    fn garbage_is_a_protocol_error() {
        let mut s = Session::new();
        assert!(s.on_stream(&[0, 0, 0, 1, 99], 0.0).is_err());
        assert_eq!(s.screen(), Screen::Error);
    }

    #[test]
    fn a_hidden_tab_does_not_flood_old_inputs() {
        let mut s = Session::new();
        s.on_stream(&joined().encode_frame(), 0.0).unwrap();
        let start = pqpq_protocol::RaceStart {
            start_id: 1,
            start_tick: 190,
            server_tick: 100,
            grid: vec![pqpq_protocol::GridSlot {
                player_id: PlayerId(7),
                position: [0.0, 0.0],
                direction: 0.0,
            }],
        };
        s.on_stream(
            &ServerMessage::RaceStart {
                start,
                room_revision: 2,
            }
            .encode_frame(),
            0.0,
        )
        .unwrap();
        s.update(0.0);
        s.update(60_000.0);
        let mut n = 0;
        while s.take_datagram().is_some() {
            n += 1;
        }
        assert!(n <= 3, "{n}");
    }
}
