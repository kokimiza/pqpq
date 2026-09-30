use std::fmt;

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::{
    DATAGRAM_VERSION, MAX_FRAME_LEN, MAX_PENDING_SNAPSHOTS, MAX_SNAPSHOT_FRAGMENTS,
    MAX_SNAPSHOT_LEN, SNAPSHOT_TIMEOUT_MS,
};

pub(crate) const KIND_INPUT: u8 = 1;
pub(crate) const KIND_SNAPSHOT: u8 = 2;
/// Fixed-width, big-endian: version, kind, tick, revision, index, count.
/// Kept hand-written because its size decides how much payload fits.
const FRAGMENT_HEADER_LEN: usize = 1 + 1 + 8 + 8 + 1 + 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecodeError {
    Truncated,
    TrailingBytes,
    FrameTooLarge,
    Invalid(&'static str),
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecodeError::Truncated => f.write_str("message truncated"),
            DecodeError::TrailingBytes => f.write_str("unexpected trailing bytes"),
            DecodeError::FrameTooLarge => f.write_str("frame too large"),
            DecodeError::Invalid(what) => write!(f, "invalid {what}"),
        }
    }
}

impl std::error::Error for DecodeError {}

/// Decodes one postcard body; every byte must belong to the message.
pub(crate) fn decode_body<T: DeserializeOwned>(body: &[u8]) -> Result<T, DecodeError> {
    let (value, rest) = postcard::take_from_bytes(body).map_err(|e| match e {
        postcard::Error::DeserializeUnexpectedEnd => DecodeError::Truncated,
        _ => DecodeError::Invalid("message"),
    })?;
    if rest.is_empty() {
        Ok(value)
    } else {
        Err(DecodeError::TrailingBytes)
    }
}

/// Appends the postcard body of `value` to `prefix`.
pub(crate) fn encode_after<T: Serialize>(prefix: Vec<u8>, value: &T) -> Vec<u8> {
    // Only fails for types serde cannot represent, which none of ours are.
    postcard::to_extend(value, prefix).expect("protocol types always serialize")
}

/// `u32` big-endian length prefix + postcard body, for the control stream.
pub(crate) fn encode_frame<T: Serialize>(value: &T) -> Vec<u8> {
    let mut out = encode_after(vec![0; 4], value);
    let len = (out.len() - 4) as u32;
    out[..4].copy_from_slice(&len.to_be_bytes());
    out
}

/// Every datagram starts with `[DATAGRAM_VERSION, kind]`.
pub(crate) fn datagram_body(datagram: &[u8], kind: u8) -> Result<&[u8], DecodeError> {
    match datagram {
        [DATAGRAM_VERSION, k, body @ ..] if *k == kind => Ok(body),
        [_, _, ..] => Err(DecodeError::Invalid("datagram header")),
        _ => Err(DecodeError::Truncated),
    }
}

/// Splits a byte stream into message bodies. Handles split and coalesced
/// reads; rejects oversized lengths before buffering the body.
#[derive(Debug, Default)]
pub struct FrameDecoder {
    buf: Vec<u8>,
    overflowed: bool,
}

impl FrameDecoder {
    pub fn push(&mut self, bytes: &[u8]) {
        // Bound coalesced reads as well as individual frame lengths. In
        // particular, native QUIC may expose several reads before decoding.
        const MAX_BUFFER: usize = 4 * MAX_FRAME_LEN + 4;
        if self.overflowed || bytes.len() > MAX_BUFFER.saturating_sub(self.buf.len()) {
            self.buf.clear();
            self.overflowed = true;
            return;
        }
        self.buf.extend_from_slice(bytes);
    }

    /// Next complete body, `Ok(None)` when more bytes are needed.
    pub fn next_frame(&mut self) -> Result<Option<Vec<u8>>, DecodeError> {
        if self.overflowed {
            return Err(DecodeError::FrameTooLarge);
        }
        if self.buf.len() < 4 {
            return Ok(None);
        }
        let len = u32::from_be_bytes(self.buf[..4].try_into().unwrap()) as usize;
        if len > MAX_FRAME_LEN {
            return Err(DecodeError::FrameTooLarge);
        }
        if self.buf.len() < 4 + len {
            return Ok(None);
        }
        let body = self.buf[4..4 + len].to_vec();
        self.buf.drain(..4 + len);
        Ok(Some(body))
    }
}

/// One snapshot datagram.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotFragment {
    pub tick: u64,
    pub room_revision: u64,
    pub index: u8,
    pub count: u8,
    pub payload: Vec<u8>,
}

impl SnapshotFragment {
    pub fn decode(datagram: &[u8]) -> Result<Self, DecodeError> {
        datagram_body(datagram, KIND_SNAPSHOT)?;
        if datagram.len() < FRAGMENT_HEADER_LEN {
            return Err(DecodeError::Truncated);
        }
        let (header, payload) = datagram.split_at(FRAGMENT_HEADER_LEN);
        let u64_at = |i: usize| u64::from_be_bytes(header[i..i + 8].try_into().unwrap());
        let (index, count) = (header[18], header[19]);
        if count == 0 || count as usize > MAX_SNAPSHOT_FRAGMENTS || index >= count {
            return Err(DecodeError::Invalid("fragment index"));
        }
        Ok(SnapshotFragment {
            tick: u64_at(2),
            room_revision: u64_at(10),
            index,
            count,
            payload: payload.to_vec(),
        })
    }
}

/// Splits an encoded snapshot into datagrams of at most `max_payload` bytes.
/// `None` when it cannot be delivered within the fragment limits.
pub fn fragment_snapshot(
    tick: u64,
    room_revision: u64,
    body: &[u8],
    max_payload: usize,
) -> Option<Vec<Vec<u8>>> {
    let chunk = max_payload
        .checked_sub(FRAGMENT_HEADER_LEN)
        .filter(|c| *c > 0)?;
    if body.len() > MAX_SNAPSHOT_LEN {
        return None;
    }
    let count = body.len().div_ceil(chunk).max(1);
    if count > MAX_SNAPSHOT_FRAGMENTS {
        return None;
    }
    let parts: Vec<&[u8]> = if body.is_empty() {
        vec![&[]]
    } else {
        body.chunks(chunk).collect()
    };
    Some(
        parts
            .into_iter()
            .enumerate()
            .map(|(i, part)| {
                let mut d = Vec::with_capacity(FRAGMENT_HEADER_LEN + part.len());
                d.extend([DATAGRAM_VERSION, KIND_SNAPSHOT]);
                d.extend(tick.to_be_bytes());
                d.extend(room_revision.to_be_bytes());
                d.extend([i as u8, count as u8]);
                d.extend_from_slice(part);
                d
            })
            .collect(),
    )
}

struct Pending {
    tick: u64,
    room_revision: u64,
    parts: Vec<Option<Vec<u8>>>,
    bytes: usize,
    started_ms: f64,
}

/// Reassembles fragments. Keeps at most three generations for 250 ms, never
/// waits for a lost one, and discards anything older than the last completed.
#[derive(Default)]
pub struct SnapshotAssembler {
    pending: Vec<Pending>,
    done_tick: Option<u64>,
}

impl SnapshotAssembler {
    /// Returns the complete snapshot body once every fragment arrived.
    pub fn push(&mut self, frag: SnapshotFragment, now_ms: f64) -> Option<Vec<u8>> {
        self.pending
            .retain(|p| now_ms - p.started_ms <= SNAPSHOT_TIMEOUT_MS);
        if self.done_tick.is_some_and(|t| frag.tick <= t) {
            return None;
        }
        let pos = match self
            .pending
            .iter()
            .position(|p| p.tick == frag.tick && p.room_revision == frag.room_revision)
        {
            Some(i) if self.pending[i].parts.len() == frag.count as usize => i,
            Some(_) => return None,
            None => {
                if self.pending.len() >= MAX_PENDING_SNAPSHOTS {
                    let oldest = (0..self.pending.len())
                        .min_by_key(|&i| self.pending[i].tick)
                        .unwrap();
                    self.pending.remove(oldest);
                }
                self.pending.push(Pending {
                    tick: frag.tick,
                    room_revision: frag.room_revision,
                    parts: vec![None; frag.count as usize],
                    bytes: 0,
                    started_ms: now_ms,
                });
                self.pending.len() - 1
            },
        };
        let p = &mut self.pending[pos];
        let slot = &mut p.parts[frag.index as usize];
        if slot.is_none() {
            p.bytes += frag.payload.len();
            *slot = Some(frag.payload);
        }
        if p.bytes > MAX_SNAPSHOT_LEN {
            self.pending.remove(pos);
            return None;
        }
        if p.parts.iter().any(Option::is_none) {
            return None;
        }
        let p = self.pending.remove(pos);
        self.done_tick = Some(p.tick);
        self.pending.retain(|q| q.tick > p.tick);
        Some(p.parts.into_iter().flatten().flatten().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_survive_split_and_coalesced_reads() {
        let a = encode_frame(&1u8);
        let b = encode_frame(&"hello"); // varint length + 5 bytes
        let joined: Vec<u8> = a.iter().chain(&b).copied().collect();
        let mut d = FrameDecoder::default();
        for byte in &joined[..3] {
            d.push(&[*byte]);
            assert_eq!(d.next_frame(), Ok(None));
        }
        d.push(&joined[3..]);
        assert_eq!(d.next_frame(), Ok(Some(vec![1])));
        assert_eq!(d.next_frame().unwrap().unwrap().len(), 6);
        assert_eq!(d.next_frame(), Ok(None));
    }

    #[test]
    fn oversized_frame_is_rejected_before_buffering() {
        let mut d = FrameDecoder::default();
        d.push(&((MAX_FRAME_LEN as u32 + 1).to_be_bytes()));
        assert_eq!(d.next_frame(), Err(DecodeError::FrameTooLarge));
    }

    #[test]
    fn flooding_before_decode_does_not_grow_the_buffer() {
        let mut d = FrameDecoder::default();
        for _ in 0..100 {
            d.push(&[0; 8192]);
        }
        assert!(d.buf.is_empty());
        assert_eq!(d.next_frame(), Err(DecodeError::FrameTooLarge));
    }

    #[test]
    fn datagram_header_is_checked() {
        assert_eq!(datagram_body(&[DATAGRAM_VERSION, KIND_INPUT, 9], KIND_INPUT), Ok(&[9][..]));
        assert!(datagram_body(&[DATAGRAM_VERSION + 1, KIND_INPUT], KIND_INPUT).is_err());
        assert!(datagram_body(&[DATAGRAM_VERSION, KIND_SNAPSHOT], KIND_INPUT).is_err());
        assert_eq!(datagram_body(&[DATAGRAM_VERSION], KIND_INPUT), Err(DecodeError::Truncated));
        assert_eq!(SnapshotFragment::decode(&[DATAGRAM_VERSION, KIND_SNAPSHOT, 0]), Err(DecodeError::Truncated));
    }

    #[test]
    fn trailing_bytes_are_rejected() {
        assert_eq!(decode_body::<u8>(&[1]), Ok(1));
        assert_eq!(decode_body::<u8>(&[1, 2]), Err(DecodeError::TrailingBytes));
        assert_eq!(decode_body::<u16>(&[0x80]), Err(DecodeError::Truncated));
    }

    #[test]
    fn fragments_reassemble_out_of_order() {
        let body: Vec<u8> = (0..1000u32).map(|i| i as u8).collect();
        let frags = fragment_snapshot(7, 3, &body, 300).unwrap();
        assert_eq!(frags.len(), 4);
        let mut a = SnapshotAssembler::default();
        let mut out = None;
        for f in frags.iter().rev() {
            out = a.push(SnapshotFragment::decode(f).unwrap(), 0.0);
        }
        assert_eq!(out, Some(body));
    }

    #[test]
    fn lost_fragment_does_not_block_newer_snapshots() {
        let mut a = SnapshotAssembler::default();
        let old = fragment_snapshot(1, 1, &[1; 500], 300).unwrap();
        a.push(SnapshotFragment::decode(&old[0]).unwrap(), 0.0);
        let new = fragment_snapshot(2, 1, &[2; 10], 300).unwrap();
        assert!(
            a.push(SnapshotFragment::decode(&new[0]).unwrap(), 10.0)
                .is_some()
        );
        // The late rest of tick 1 is now useless.
        assert!(
            a.push(SnapshotFragment::decode(&old[1]).unwrap(), 20.0)
                .is_none()
        );
        assert!(a.pending.is_empty());
    }

    #[test]
    fn stale_partial_snapshots_expire() {
        let mut a = SnapshotAssembler::default();
        let frags = fragment_snapshot(1, 1, &[1; 500], 300).unwrap();
        a.push(SnapshotFragment::decode(&frags[0]).unwrap(), 0.0);
        assert!(
            a.push(SnapshotFragment::decode(&frags[1]).unwrap(), 300.0)
                .is_none()
        );
    }

    #[test]
    fn too_many_fragments_is_refused() {
        assert!(fragment_snapshot(1, 1, &[0; MAX_SNAPSHOT_LEN], 100).is_none());
        assert!(fragment_snapshot(1, 1, &[0; 10], FRAGMENT_HEADER_LEN).is_none());
    }
}
