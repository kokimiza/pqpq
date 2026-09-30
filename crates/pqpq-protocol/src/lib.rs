//! Transport-independent protocol shared by server, terminal and browser.
//!
//! All integers are big-endian (network byte order). Stream messages are
//! framed as `u32 length + body`; every datagram starts with a version byte
//! and a kind byte.

mod codec;
mod messages;
mod types;
mod view;

pub use codec::{
    DecodeError, FrameDecoder, SnapshotAssembler, SnapshotFragment, fragment_snapshot,
};
pub use messages::*;
pub use types::*;
pub use view::RoomView;

/// Application protocol version, independent of QUIC and HTTP/3 versions.
pub const PROTOCOL_VERSION: u32 = 1;
/// Version byte at the start of every datagram.
pub const DATAGRAM_VERSION: u8 = 1;
/// ALPN for the native QUIC transport.
pub const NATIVE_ALPN: &[u8] = b"pqpq/1";
/// WebTransport session path on the server.
pub const WEBTRANSPORT_PATH: &str = "/transport";
/// Upper bound of one stream message body.
pub const MAX_FRAME_LEN: usize = 64 * 1024;
/// Upper bound of one reassembled snapshot.
pub const MAX_SNAPSHOT_LEN: usize = 16 * 1024;
pub const MAX_SNAPSHOT_FRAGMENTS: usize = 16;
/// Snapshot generations reassembled at the same time.
pub const MAX_PENDING_SNAPSHOTS: usize = 3;
/// Incomplete snapshots are dropped after this long.
pub const SNAPSHOT_TIMEOUT_MS: f64 = 250.0;
