use crate::codec::{DecodeError, KIND_INPUT, Reader, Writer, frame};
use crate::types::*;
use crate::{DATAGRAM_VERSION, MAX_ROOM_ID_LEN, MAX_USERNAME_BYTES};

const MAX_ERROR_MESSAGE: usize = 256;
const MAX_ROSTER: usize = 1024;
const MAX_CARS: usize = 255;

/// Current controls. Steering is -1 (right), 0 or 1 (left).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Controls {
    pub throttle: bool,
    pub brake: bool,
    pub steering: i8,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Join {
    pub protocol_version: u32,
    pub username: String,
    pub room_id: String,
    pub course_id: u32,
    pub course_version: u32,
    pub physics_version: u32,
}

/// Client to server, over the reliable control stream.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ClientMessage {
    Join(Join),
    Ready,
    Leave,
    Ping { nonce: u64 },
    Pong { nonce: u64 },
}

/// Client to server, unreliable. Carries only controls, never positions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InputDatagram {
    pub sequence: u64,
    pub controls: Controls,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RosterEntry {
    pub player_id: PlayerId,
    pub username: String,
    pub role: Role,
    pub ready: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridSlot {
    pub player_id: PlayerId,
    pub position: [f32; 2],
    pub direction: f32,
}

/// A scheduled start. Entrants are fixed by `grid`.
#[derive(Clone, Debug, PartialEq)]
pub struct RaceStart {
    pub start_id: u64,
    pub start_tick: u64,
    /// Server tick when this was sent, for the countdown.
    pub server_tick: u64,
    pub grid: Vec<GridSlot>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResultEntry {
    pub player_id: PlayerId,
    pub username: String,
    pub status: CarStatus,
    pub laps: u8,
    /// Race time for finishers.
    pub finish_time_ms: Option<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Joined {
    pub player_id: PlayerId,
    pub room_id: String,
    pub role: Role,
    pub roster: Vec<RosterEntry>,
    pub room_revision: u64,
    pub phase: RacePhase,
    pub server_tick: u64,
    pub start: Option<RaceStart>,
    pub course_id: u32,
    pub course_version: u32,
    pub physics_version: u32,
    pub tick_hz: u16,
    pub laps: u8,
    /// Racers needed before a start, shown in the waiting room.
    pub min_racers: u8,
    /// Filled when joining a finished room.
    pub results: Vec<ResultEntry>,
}

/// Server to client, over the reliable control stream.
#[derive(Clone, Debug, PartialEq)]
pub enum ServerMessage {
    Joined(Joined),
    PlayerJoined {
        entry: RosterEntry,
        room_revision: u64,
    },
    PlayerLeft {
        player_id: PlayerId,
        reason: LeaveReason,
        room_revision: u64,
    },
    ReadyChanged {
        player_id: PlayerId,
        ready: bool,
        room_revision: u64,
    },
    RaceStart {
        start: RaceStart,
        room_revision: u64,
    },
    RaceStartCancelled {
        start_id: u64,
        reason: CancelReason,
        room_revision: u64,
    },
    RaceFinished {
        end_tick: u64,
        results: Vec<ResultEntry>,
        room_revision: u64,
    },
    Error {
        code: ErrorCode,
        message: String,
        fatal: bool,
    },
    Ping {
        nonce: u64,
    },
    Pong {
        nonce: u64,
    },
}

impl ServerMessage {
    pub fn room_revision(&self) -> Option<u64> {
        match self {
            ServerMessage::Joined(j) => Some(j.room_revision),
            ServerMessage::PlayerJoined { room_revision, .. }
            | ServerMessage::PlayerLeft { room_revision, .. }
            | ServerMessage::ReadyChanged { room_revision, .. }
            | ServerMessage::RaceStart { room_revision, .. }
            | ServerMessage::RaceStartCancelled { room_revision, .. }
            | ServerMessage::RaceFinished { room_revision, .. } => Some(*room_revision),
            _ => None,
        }
    }

    pub fn error(code: ErrorCode, fatal: bool) -> Self {
        ServerMessage::Error {
            code,
            message: code.message().to_owned(),
            fatal,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CarSnapshot {
    pub player_id: PlayerId,
    pub position: [f32; 2],
    pub velocity: [f32; 2],
    pub direction: f32,
    pub laps: u8,
    pub next_checkpoint: u8,
    /// 1-based position among all entrants.
    pub rank: u8,
    pub status: CarStatus,
}

/// Complete world state of one room at one tick, sent unreliably.
#[derive(Clone, Debug, PartialEq)]
pub struct GameSnapshot {
    pub tick: u64,
    pub room_revision: u64,
    pub phase: RacePhase,
    pub start_id: u64,
    pub start_tick: u64,
    /// Entrants fixed at start, the denominator of the position display.
    pub entrants: u8,
    pub cars: Vec<CarSnapshot>,
    /// Receiver-specific: newest input sequence consumed by the server.
    pub last_processed_input: u64,
}

trait Wire: Sized {
    fn put(&self, w: &mut Writer);
    fn get(r: &mut Reader) -> Result<Self, DecodeError>;
}

macro_rules! wire_enums {
    ($($t:ident),*) => {$(
        impl Wire for $t {
            fn put(&self, w: &mut Writer) {
                w.u8(*self as u8);
            }
            fn get(r: &mut Reader) -> Result<Self, DecodeError> {
                $t::from_u8(r.u8()?).ok_or(DecodeError::Invalid(stringify!($t)))
            }
        }
    )*};
}
wire_enums!(
    Role,
    RacePhase,
    CarStatus,
    LeaveReason,
    CancelReason,
    ErrorCode
);

impl Wire for PlayerId {
    fn put(&self, w: &mut Writer) {
        w.u64(self.0);
    }
    fn get(r: &mut Reader) -> Result<Self, DecodeError> {
        Ok(PlayerId(r.u64()?))
    }
}

fn put_list<T: Wire>(w: &mut Writer, items: &[T]) {
    w.u16(items.len() as u16);
    items.iter().for_each(|i| i.put(w));
}

fn get_list<T: Wire>(r: &mut Reader, max: usize) -> Result<Vec<T>, DecodeError> {
    let n = r.u16()? as usize;
    if n > max {
        return Err(DecodeError::Invalid("list length"));
    }
    // Every element needs bytes, so a lying count fails with Truncated early.
    let mut out = Vec::new();
    for _ in 0..n {
        out.push(T::get(r)?);
    }
    Ok(out)
}

impl Wire for RosterEntry {
    fn put(&self, w: &mut Writer) {
        self.player_id.put(w);
        w.str(&self.username);
        self.role.put(w);
        w.bool(self.ready);
    }
    fn get(r: &mut Reader) -> Result<Self, DecodeError> {
        Ok(RosterEntry {
            player_id: PlayerId::get(r)?,
            username: r.str(MAX_USERNAME_BYTES)?,
            role: Role::get(r)?,
            ready: r.bool()?,
        })
    }
}

impl Wire for GridSlot {
    fn put(&self, w: &mut Writer) {
        self.player_id.put(w);
        w.f32(self.position[0]);
        w.f32(self.position[1]);
        w.f32(self.direction);
    }
    fn get(r: &mut Reader) -> Result<Self, DecodeError> {
        Ok(GridSlot {
            player_id: PlayerId::get(r)?,
            position: [r.f32()?, r.f32()?],
            direction: r.f32()?,
        })
    }
}

impl Wire for RaceStart {
    fn put(&self, w: &mut Writer) {
        w.u64(self.start_id);
        w.u64(self.start_tick);
        w.u64(self.server_tick);
        put_list(w, &self.grid);
    }
    fn get(r: &mut Reader) -> Result<Self, DecodeError> {
        Ok(RaceStart {
            start_id: r.u64()?,
            start_tick: r.u64()?,
            server_tick: r.u64()?,
            grid: get_list(r, MAX_CARS)?,
        })
    }
}

impl Wire for ResultEntry {
    fn put(&self, w: &mut Writer) {
        self.player_id.put(w);
        w.str(&self.username);
        self.status.put(w);
        w.u8(self.laps);
        w.u32(self.finish_time_ms.unwrap_or(u32::MAX));
    }
    fn get(r: &mut Reader) -> Result<Self, DecodeError> {
        Ok(ResultEntry {
            player_id: PlayerId::get(r)?,
            username: r.str(MAX_USERNAME_BYTES)?,
            status: CarStatus::get(r)?,
            laps: r.u8()?,
            finish_time_ms: Some(r.u32()?).filter(|t| *t != u32::MAX),
        })
    }
}

impl Wire for CarSnapshot {
    fn put(&self, w: &mut Writer) {
        self.player_id.put(w);
        for v in [
            self.position[0],
            self.position[1],
            self.velocity[0],
            self.velocity[1],
            self.direction,
        ] {
            w.f32(v);
        }
        w.u8(self.laps);
        w.u8(self.next_checkpoint);
        w.u8(self.rank);
        self.status.put(w);
    }
    fn get(r: &mut Reader) -> Result<Self, DecodeError> {
        Ok(CarSnapshot {
            player_id: PlayerId::get(r)?,
            position: [r.f32()?, r.f32()?],
            velocity: [r.f32()?, r.f32()?],
            direction: r.f32()?,
            laps: r.u8()?,
            next_checkpoint: r.u8()?,
            rank: r.u8()?,
            status: CarStatus::get(r)?,
        })
    }
}

impl ClientMessage {
    /// Length-prefixed bytes for the control stream.
    pub fn encode_frame(&self) -> Vec<u8> {
        frame(|w| match self {
            ClientMessage::Join(j) => {
                w.u8(1);
                w.u32(j.protocol_version);
                w.str(&j.username);
                w.str(&j.room_id);
                w.u32(j.course_id);
                w.u32(j.course_version);
                w.u32(j.physics_version);
            },
            ClientMessage::Ready => w.u8(2),
            ClientMessage::Leave => w.u8(3),
            ClientMessage::Ping { nonce } => {
                w.u8(4);
                w.u64(*nonce);
            },
            ClientMessage::Pong { nonce } => {
                w.u8(5);
                w.u64(*nonce);
            },
        })
    }

    /// Decodes one frame body (without the length prefix).
    pub fn decode(body: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader::new(body);
        let msg = match r.u8()? {
            1 => ClientMessage::Join(Join {
                protocol_version: r.u32()?,
                // Length is checked here, content by validate_username.
                username: r.str(MAX_USERNAME_BYTES)?,
                room_id: r.str(MAX_ROOM_ID_LEN)?,
                course_id: r.u32()?,
                course_version: r.u32()?,
                physics_version: r.u32()?,
            }),
            2 => ClientMessage::Ready,
            3 => ClientMessage::Leave,
            4 => ClientMessage::Ping { nonce: r.u64()? },
            5 => ClientMessage::Pong { nonce: r.u64()? },
            _ => return Err(DecodeError::Invalid("client message type")),
        };
        r.finish()?;
        Ok(msg)
    }
}

impl ServerMessage {
    pub fn encode_frame(&self) -> Vec<u8> {
        frame(|w| match self {
            ServerMessage::Joined(j) => {
                w.u8(1);
                j.player_id.put(w);
                w.str(&j.room_id);
                j.role.put(w);
                put_list(w, &j.roster);
                w.u64(j.room_revision);
                j.phase.put(w);
                w.u64(j.server_tick);
                w.bool(j.start.is_some());
                if let Some(s) = &j.start {
                    s.put(w);
                }
                w.u32(j.course_id);
                w.u32(j.course_version);
                w.u32(j.physics_version);
                w.u16(j.tick_hz);
                w.u8(j.laps);
                w.u8(j.min_racers);
                put_list(w, &j.results);
            },
            ServerMessage::PlayerJoined {
                entry,
                room_revision,
            } => {
                w.u8(2);
                entry.put(w);
                w.u64(*room_revision);
            },
            ServerMessage::PlayerLeft {
                player_id,
                reason,
                room_revision,
            } => {
                w.u8(3);
                player_id.put(w);
                reason.put(w);
                w.u64(*room_revision);
            },
            ServerMessage::ReadyChanged {
                player_id,
                ready,
                room_revision,
            } => {
                w.u8(4);
                player_id.put(w);
                w.bool(*ready);
                w.u64(*room_revision);
            },
            ServerMessage::RaceStart {
                start,
                room_revision,
            } => {
                w.u8(5);
                start.put(w);
                w.u64(*room_revision);
            },
            ServerMessage::RaceStartCancelled {
                start_id,
                reason,
                room_revision,
            } => {
                w.u8(6);
                w.u64(*start_id);
                reason.put(w);
                w.u64(*room_revision);
            },
            ServerMessage::RaceFinished {
                end_tick,
                results,
                room_revision,
            } => {
                w.u8(7);
                w.u64(*end_tick);
                put_list(w, results);
                w.u64(*room_revision);
            },
            ServerMessage::Error {
                code,
                message,
                fatal,
            } => {
                w.u8(8);
                code.put(w);
                w.str(truncate(message, MAX_ERROR_MESSAGE));
                w.bool(*fatal);
            },
            ServerMessage::Ping { nonce } => {
                w.u8(9);
                w.u64(*nonce);
            },
            ServerMessage::Pong { nonce } => {
                w.u8(10);
                w.u64(*nonce);
            },
        })
    }

    pub fn decode(body: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader::new(body);
        let msg = match r.u8()? {
            1 => ServerMessage::Joined(Joined {
                player_id: PlayerId::get(&mut r)?,
                room_id: r.str(MAX_ROOM_ID_LEN)?,
                role: Role::get(&mut r)?,
                roster: get_list(&mut r, MAX_ROSTER)?,
                room_revision: r.u64()?,
                phase: RacePhase::get(&mut r)?,
                server_tick: r.u64()?,
                start: if r.bool()? {
                    Some(RaceStart::get(&mut r)?)
                } else {
                    None
                },
                course_id: r.u32()?,
                course_version: r.u32()?,
                physics_version: r.u32()?,
                tick_hz: r.u16()?,
                laps: r.u8()?,
                min_racers: r.u8()?,
                results: get_list(&mut r, MAX_CARS)?,
            }),
            2 => ServerMessage::PlayerJoined {
                entry: RosterEntry::get(&mut r)?,
                room_revision: r.u64()?,
            },
            3 => ServerMessage::PlayerLeft {
                player_id: PlayerId::get(&mut r)?,
                reason: LeaveReason::get(&mut r)?,
                room_revision: r.u64()?,
            },
            4 => ServerMessage::ReadyChanged {
                player_id: PlayerId::get(&mut r)?,
                ready: r.bool()?,
                room_revision: r.u64()?,
            },
            5 => ServerMessage::RaceStart {
                start: RaceStart::get(&mut r)?,
                room_revision: r.u64()?,
            },
            6 => ServerMessage::RaceStartCancelled {
                start_id: r.u64()?,
                reason: CancelReason::get(&mut r)?,
                room_revision: r.u64()?,
            },
            7 => ServerMessage::RaceFinished {
                end_tick: r.u64()?,
                results: get_list(&mut r, MAX_CARS)?,
                room_revision: r.u64()?,
            },
            8 => ServerMessage::Error {
                code: ErrorCode::get(&mut r)?,
                message: r.str(MAX_ERROR_MESSAGE)?,
                fatal: r.bool()?,
            },
            9 => ServerMessage::Ping { nonce: r.u64()? },
            10 => ServerMessage::Pong { nonce: r.u64()? },
            _ => return Err(DecodeError::Invalid("server message type")),
        };
        r.finish()?;
        Ok(msg)
    }
}

fn truncate(s: &str, max: usize) -> &str {
    let mut end = s.len().min(max);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

impl InputDatagram {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::default();
        w.u8(DATAGRAM_VERSION);
        w.u8(KIND_INPUT);
        w.u64(self.sequence);
        w.u8(self.controls.throttle as u8 | (self.controls.brake as u8) << 1);
        w.u8(self.controls.steering as u8);
        w.0
    }

    pub fn decode(datagram: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader::new(datagram);
        if r.u8()? != DATAGRAM_VERSION {
            return Err(DecodeError::Invalid("datagram version"));
        }
        if r.u8()? != KIND_INPUT {
            return Err(DecodeError::Invalid("datagram kind"));
        }
        let sequence = r.u64()?;
        let flags = r.u8()?;
        let steering = r.u8()? as i8;
        r.finish()?;
        if flags > 0b11 || !(-1..=1).contains(&steering) || sequence == 0 {
            return Err(DecodeError::Invalid("input"));
        }
        Ok(InputDatagram {
            sequence,
            controls: Controls {
                throttle: flags & 1 != 0,
                brake: flags & 2 != 0,
                steering,
            },
        })
    }
}

impl GameSnapshot {
    /// Body before fragmentation.
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::default();
        w.u64(self.tick);
        w.u64(self.room_revision);
        self.phase.put(&mut w);
        w.u64(self.start_id);
        w.u64(self.start_tick);
        w.u8(self.entrants);
        put_list(&mut w, &self.cars);
        w.u64(self.last_processed_input);
        w.0
    }

    pub fn decode(body: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader::new(body);
        let snap = GameSnapshot {
            tick: r.u64()?,
            room_revision: r.u64()?,
            phase: RacePhase::get(&mut r)?,
            start_id: r.u64()?,
            start_tick: r.u64()?,
            entrants: r.u8()?,
            cars: get_list(&mut r, MAX_CARS)?,
            last_processed_input: r.u64()?,
        };
        r.finish()?;
        Ok(snap)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        FrameDecoder, PROTOCOL_VERSION, SnapshotAssembler, SnapshotFragment, fragment_snapshot,
    };

    fn roundtrip_server(msg: ServerMessage) {
        let bytes = msg.encode_frame();
        let mut d = FrameDecoder::default();
        d.push(&bytes);
        assert_eq!(
            ServerMessage::decode(&d.next_frame().unwrap().unwrap()).unwrap(),
            msg
        );
    }

    fn join() -> ClientMessage {
        ClientMessage::Join(Join {
            protocol_version: PROTOCOL_VERSION,
            username: "foo".into(),
            room_id: "1234".into(),
            course_id: 1,
            course_version: 1,
            physics_version: 1,
        })
    }

    /// Known bytes: also checked against the WASM build by web/check.mjs.
    pub const JOIN_FRAME: [u8; 32] = [
        0, 0, 0, 28, 1, 0, 0, 0, 1, 0, 3, b'f', b'o', b'o', 0, 4, b'1', b'2', b'3', b'4', 0, 0, 0,
        1, 0, 0, 0, 1, 0, 0, 0, 1,
    ];

    #[test]
    fn join_has_fixed_bytes() {
        assert_eq!(join().encode_frame(), JOIN_FRAME);
        assert_eq!(ClientMessage::decode(&JOIN_FRAME[4..]).unwrap(), join());
    }

    #[test]
    fn input_has_fixed_bytes() {
        let i = InputDatagram {
            sequence: 258,
            controls: Controls {
                throttle: true,
                brake: false,
                steering: -1,
            },
        };
        let bytes = i.encode();
        assert_eq!(bytes, [1, 1, 0, 0, 0, 0, 0, 0, 1, 2, 1, 0xff]);
        assert_eq!(InputDatagram::decode(&bytes).unwrap(), i);
    }

    #[test]
    fn invalid_inputs_are_rejected() {
        let mut bad = InputDatagram {
            sequence: 1,
            controls: Controls::default(),
        }
        .encode();
        bad[11] = 2; // steering out of range
        assert!(InputDatagram::decode(&bad).is_err());
        let zero = InputDatagram {
            sequence: 0,
            controls: Controls::default(),
        }
        .encode();
        assert!(InputDatagram::decode(&zero).is_err());
        assert!(ClientMessage::decode(&[2, 0]).is_err()); // trailing byte
        assert!(ClientMessage::decode(&[1, 0, 0]).is_err()); // truncated
        let mut invalid_utf8 = join().encode_frame()[4..].to_vec();
        invalid_utf8[7] = 0xff;
        assert!(ClientMessage::decode(&invalid_utf8).is_err());
    }

    #[test]
    fn server_messages_roundtrip() {
        let entry = RosterEntry {
            player_id: PlayerId(2),
            username: "ふー".into(),
            role: Role::Racer,
            ready: true,
        };
        let start = RaceStart {
            start_id: 5,
            start_tick: 90,
            server_tick: 0,
            grid: vec![GridSlot {
                player_id: PlayerId(2),
                position: [1.5, -2.0],
                direction: 0.5,
            }],
        };
        let result = ResultEntry {
            player_id: PlayerId(2),
            username: "foo".into(),
            status: CarStatus::Finished,
            laps: 3,
            finish_time_ms: Some(42_300),
        };
        let dnf = ResultEntry {
            status: CarStatus::Dnf,
            finish_time_ms: None,
            ..result.clone()
        };
        roundtrip_server(ServerMessage::Joined(Joined {
            player_id: PlayerId(2),
            room_id: "1234".into(),
            role: Role::Spectator,
            roster: vec![entry.clone()],
            room_revision: 9,
            phase: RacePhase::Finished,
            server_tick: 1000,
            start: Some(start.clone()),
            course_id: 1,
            course_version: 1,
            physics_version: 1,
            tick_hz: 30,
            laps: 3,
            min_racers: 2,
            results: vec![result.clone(), dnf],
        }));
        roundtrip_server(ServerMessage::PlayerJoined {
            entry,
            room_revision: 1,
        });
        roundtrip_server(ServerMessage::PlayerLeft {
            player_id: PlayerId(1),
            reason: LeaveReason::Disconnected,
            room_revision: 2,
        });
        roundtrip_server(ServerMessage::ReadyChanged {
            player_id: PlayerId(1),
            ready: true,
            room_revision: 3,
        });
        roundtrip_server(ServerMessage::RaceStart {
            start,
            room_revision: 4,
        });
        roundtrip_server(ServerMessage::RaceStartCancelled {
            start_id: 5,
            reason: CancelReason::RosterChanged,
            room_revision: 5,
        });
        roundtrip_server(ServerMessage::RaceFinished {
            end_tick: 99,
            results: vec![result],
            room_revision: 6,
        });
        roundtrip_server(ServerMessage::error(ErrorCode::RoomFull, true));
        roundtrip_server(ServerMessage::Ping { nonce: 7 });
        roundtrip_server(ServerMessage::Pong { nonce: 8 });
    }

    #[test]
    fn snapshot_roundtrips_through_fragments() {
        let car = CarSnapshot {
            player_id: PlayerId(1),
            position: [1.0, 2.0],
            velocity: [3.0, -4.0],
            direction: 0.25,
            laps: 1,
            next_checkpoint: 2,
            rank: 1,
            status: CarStatus::Racing,
        };
        let snap = GameSnapshot {
            tick: 100,
            room_revision: 4,
            phase: RacePhase::Racing,
            start_id: 1,
            start_tick: 90,
            entrants: 40,
            cars: vec![car; 40],
            last_processed_input: 77,
        };
        let body = snap.encode();
        let frags = fragment_snapshot(snap.tick, snap.room_revision, &body, 1200).unwrap();
        assert!(frags.len() > 1);
        let mut a = SnapshotAssembler::default();
        let out = frags
            .iter()
            .filter_map(|f| a.push(SnapshotFragment::decode(f).unwrap(), 0.0))
            .last();
        assert_eq!(GameSnapshot::decode(&out.unwrap()).unwrap(), snap);
    }

    #[test]
    fn non_finite_floats_are_rejected() {
        let snap = GameSnapshot {
            tick: 1,
            room_revision: 1,
            phase: RacePhase::Racing,
            start_id: 1,
            start_tick: 0,
            entrants: 1,
            cars: vec![CarSnapshot {
                player_id: PlayerId(1),
                position: [f32::NAN, 0.0],
                velocity: [0.0; 2],
                direction: 0.0,
                laps: 0,
                next_checkpoint: 0,
                rank: 1,
                status: CarStatus::Racing,
            }],
            last_processed_input: 0,
        };
        assert!(GameSnapshot::decode(&snap.encode()).is_err());
    }
}
