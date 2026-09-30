//! Message types. Bodies are postcard (varint integers, little-endian floats,
//! message tags in declaration order); limits are enforced while decoding, so
//! a decoded message is always within range.

use serde::{Deserialize, Serialize};

use crate::codec::{
    DecodeError, KIND_INPUT, datagram_body, decode_body, encode_after, encode_frame,
};
use crate::types::*;
use crate::DATAGRAM_VERSION;

const MAX_ERROR_MESSAGE: usize = 256;
const MAX_ROSTER: usize = 1024;
const MAX_CARS: usize = 255;

/// Current controls. Steering is -1 (right), 0 or 1 (left).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct Controls {
    pub throttle: bool,
    pub brake: bool,
    #[serde(deserialize_with = "limit::steering")]
    pub steering: i8,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Join {
    pub protocol_version: u32,
    /// Length is checked here, content by validate_username.
    #[serde(deserialize_with = "limit::username")]
    pub username: String,
    #[serde(deserialize_with = "limit::room_id")]
    pub room_id: String,
    pub course_id: u32,
    pub course_version: u32,
    pub physics_version: u32,
}

/// Client to server, over the reliable control stream.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ClientMessage {
    Join(Join),
    Ready,
    Leave,
    Ping { nonce: u64 },
    Pong { nonce: u64 },
}

/// Client to server, unreliable. Carries only controls, never positions.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InputDatagram {
    pub sequence: u64,
    pub controls: Controls,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RosterEntry {
    pub player_id: PlayerId,
    #[serde(deserialize_with = "limit::username")]
    pub username: String,
    pub role: Role,
    pub ready: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct GridSlot {
    pub player_id: PlayerId,
    #[serde(deserialize_with = "limit::finite2")]
    pub position: [f32; 2],
    #[serde(deserialize_with = "limit::finite")]
    pub direction: f32,
}

/// A scheduled start. Entrants are fixed by `grid`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RaceStart {
    pub start_id: u64,
    pub start_tick: u64,
    /// Server tick when this was sent, for the countdown.
    pub server_tick: u64,
    #[serde(deserialize_with = "limit::cars")]
    pub grid: Vec<GridSlot>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ResultEntry {
    pub player_id: PlayerId,
    #[serde(deserialize_with = "limit::username")]
    pub username: String,
    pub status: CarStatus,
    pub laps: u8,
    /// Race time for finishers.
    pub finish_time_ms: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Joined {
    pub player_id: PlayerId,
    #[serde(deserialize_with = "limit::room_id")]
    pub room_id: String,
    pub role: Role,
    #[serde(deserialize_with = "limit::roster")]
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
    #[serde(deserialize_with = "limit::cars")]
    pub results: Vec<ResultEntry>,
}

/// Server to client, over the reliable control stream.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
        #[serde(deserialize_with = "limit::cars")]
        results: Vec<ResultEntry>,
        room_revision: u64,
    },
    Error {
        code: ErrorCode,
        #[serde(
            serialize_with = "limit::truncate_message",
            deserialize_with = "limit::message"
        )]
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

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CarSnapshot {
    pub player_id: PlayerId,
    #[serde(deserialize_with = "limit::finite2")]
    pub position: [f32; 2],
    #[serde(deserialize_with = "limit::finite2")]
    pub velocity: [f32; 2],
    #[serde(deserialize_with = "limit::finite")]
    pub direction: f32,
    pub laps: u8,
    pub next_checkpoint: u8,
    /// 1-based position among all entrants.
    pub rank: u8,
    pub status: CarStatus,
}

/// Complete world state of one room at one tick, sent unreliably.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GameSnapshot {
    pub tick: u64,
    pub room_revision: u64,
    pub phase: RacePhase,
    pub start_id: u64,
    pub start_tick: u64,
    /// Entrants fixed at start, the denominator of the position display.
    pub entrants: u8,
    #[serde(deserialize_with = "limit::cars")]
    pub cars: Vec<CarSnapshot>,
    /// Receiver-specific: newest input sequence consumed by the server.
    pub last_processed_input: u64,
}

/// Range checks run inside deserialization; a violation fails the decode.
mod limit {
    use serde::de::Error;
    use serde::{Deserialize, Deserializer, Serializer};

    use crate::{MAX_ROOM_ID_LEN, MAX_USERNAME_BYTES};

    fn bounded_str<'de, D: Deserializer<'de>>(d: D, max: usize) -> Result<String, D::Error> {
        let s = String::deserialize(d)?;
        if s.len() > max {
            return Err(D::Error::custom("string too long"));
        }
        Ok(s)
    }

    fn bounded_vec<'de, D, T>(d: D, max: usize) -> Result<Vec<T>, D::Error>
    where
        D: Deserializer<'de>,
        T: Deserialize<'de>,
    {
        let v = Vec::deserialize(d)?;
        if v.len() > max {
            return Err(D::Error::custom("list too long"));
        }
        Ok(v)
    }

    fn check<T, E: Error>(v: T, ok: bool, what: &'static str) -> Result<T, E> {
        if ok { Ok(v) } else { Err(E::custom(what)) }
    }

    pub fn username<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
        bounded_str(d, MAX_USERNAME_BYTES)
    }

    pub fn room_id<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
        bounded_str(d, MAX_ROOM_ID_LEN)
    }

    pub fn message<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
        bounded_str(d, super::MAX_ERROR_MESSAGE)
    }

    /// Cut at a char boundary so a long reason never makes the frame invalid.
    pub fn truncate_message<S: Serializer>(s: &str, ser: S) -> Result<S::Ok, S::Error> {
        let mut end = s.len().min(super::MAX_ERROR_MESSAGE);
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        ser.serialize_str(&s[..end])
    }

    pub fn roster<'de, D, T>(d: D) -> Result<Vec<T>, D::Error>
    where
        D: Deserializer<'de>,
        T: Deserialize<'de>,
    {
        bounded_vec(d, super::MAX_ROSTER)
    }

    pub fn cars<'de, D, T>(d: D) -> Result<Vec<T>, D::Error>
    where
        D: Deserializer<'de>,
        T: Deserialize<'de>,
    {
        bounded_vec(d, super::MAX_CARS)
    }

    pub fn finite<'de, D: Deserializer<'de>>(d: D) -> Result<f32, D::Error> {
        let v = f32::deserialize(d)?;
        check(v, v.is_finite(), "non-finite float")
    }

    pub fn finite2<'de, D: Deserializer<'de>>(d: D) -> Result<[f32; 2], D::Error> {
        let v = <[f32; 2]>::deserialize(d)?;
        check(v, v.iter().all(|x| x.is_finite()), "non-finite float")
    }

    pub fn steering<'de, D: Deserializer<'de>>(d: D) -> Result<i8, D::Error> {
        let v = i8::deserialize(d)?;
        check(v, (-1..=1).contains(&v), "steering out of range")
    }
}

impl ClientMessage {
    /// Length-prefixed bytes for the control stream.
    pub fn encode_frame(&self) -> Vec<u8> {
        encode_frame(self)
    }

    /// Decodes one frame body (without the length prefix).
    pub fn decode(body: &[u8]) -> Result<Self, DecodeError> {
        decode_body(body)
    }
}

impl ServerMessage {
    pub fn encode_frame(&self) -> Vec<u8> {
        encode_frame(self)
    }

    pub fn decode(body: &[u8]) -> Result<Self, DecodeError> {
        decode_body(body)
    }
}

impl InputDatagram {
    pub fn encode(&self) -> Vec<u8> {
        encode_after(vec![DATAGRAM_VERSION, KIND_INPUT], self)
    }

    pub fn decode(datagram: &[u8]) -> Result<Self, DecodeError> {
        let input: InputDatagram = decode_body(datagram_body(datagram, KIND_INPUT)?)?;
        // Sequences start at 1; 0 would sit below every processed boundary.
        if input.sequence == 0 {
            return Err(DecodeError::Invalid("input sequence"));
        }
        Ok(input)
    }
}

impl GameSnapshot {
    /// Body before fragmentation.
    pub fn encode(&self) -> Vec<u8> {
        encode_after(Vec::new(), self)
    }

    pub fn decode(body: &[u8]) -> Result<Self, DecodeError> {
        decode_body(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        FrameDecoder, MAX_USERNAME_BYTES, PROTOCOL_VERSION, SnapshotAssembler, SnapshotFragment,
        fragment_snapshot,
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
    /// u32 BE length, then postcard: tag 0 (Join), varint 1, "foo", "1234",
    /// varints 1, 1, 1.
    pub const JOIN_FRAME: [u8; 18] = [
        0, 0, 0, 14, 0, 1, 3, b'f', b'o', b'o', 4, b'1', b'2', b'3', b'4', 1, 1, 1,
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
        // Header, varint 258, throttle, brake, steering as one i8 byte.
        assert_eq!(bytes, [1, 1, 0x82, 0x02, 1, 0, 0xff]);
        assert_eq!(InputDatagram::decode(&bytes).unwrap(), i);
    }

    #[test]
    fn invalid_inputs_are_rejected() {
        let mut bad = InputDatagram {
            sequence: 1,
            controls: Controls::default(),
        }
        .encode();
        bad[5] = 2; // steering out of range
        assert!(InputDatagram::decode(&bad).is_err());
        let zero = InputDatagram {
            sequence: 0,
            controls: Controls::default(),
        }
        .encode();
        assert!(InputDatagram::decode(&zero).is_err());
        assert_eq!(ClientMessage::decode(&[1, 0]), Err(DecodeError::TrailingBytes));
        assert_eq!(ClientMessage::decode(&[0, 0]), Err(DecodeError::Truncated));
        assert!(ClientMessage::decode(&[9]).is_err()); // unknown message
        let mut invalid_utf8 = join().encode_frame()[4..].to_vec();
        invalid_utf8[3] = 0xff;
        assert!(ClientMessage::decode(&invalid_utf8).is_err());
    }

    #[test]
    fn limits_are_enforced_while_decoding() {
        let ClientMessage::Join(mut j) = join() else {
            unreachable!()
        };
        j.username = "a".repeat(MAX_USERNAME_BYTES + 1);
        let body = ClientMessage::Join(j).encode_frame();
        assert!(ClientMessage::decode(&body[4..]).is_err());
        // Unknown enum codes fail instead of mapping to some variant.
        let mut left = ServerMessage::PlayerLeft {
            player_id: PlayerId(1),
            reason: LeaveReason::Left,
            room_revision: 1,
        }
        .encode_frame();
        left[6] = 7; // after length, tag and player id comes the reason
        assert!(ServerMessage::decode(&left[4..]).is_err());
        // Explicit codes: ErrorCode::InvalidUsername is 1, not its index 0.
        let err = ServerMessage::error(ErrorCode::InvalidUsername, true).encode_frame();
        assert_eq!(err[5], 1);
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
    fn long_error_message_is_truncated_not_rejected() {
        let long = ServerMessage::Error {
            code: ErrorCode::Timeout,
            message: "あ".repeat(200),
            fatal: false,
        };
        let decoded = ServerMessage::decode(&long.encode_frame()[4..]).unwrap();
        let ServerMessage::Error { message, .. } = decoded else {
            unreachable!()
        };
        assert!(message.len() <= MAX_ERROR_MESSAGE);
        assert!(message.chars().all(|c| c == 'あ'));
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
            entrants: 60,
            cars: vec![car; 60],
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
