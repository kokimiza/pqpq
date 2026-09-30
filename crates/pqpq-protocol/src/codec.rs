use std::fmt;

use crate::{
    DATAGRAM_VERSION, MAX_FRAME_LEN, MAX_PENDING_SNAPSHOTS, MAX_SNAPSHOT_FRAGMENTS,
    MAX_SNAPSHOT_LEN, SNAPSHOT_TIMEOUT_MS,
};

pub(crate) const KIND_INPUT: u8 = 1;
pub(crate) const KIND_SNAPSHOT: u8 = 2;
/// version, kind, tick, revision, index, count
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

#[derive(Default)]
pub(crate) struct Writer(pub Vec<u8>);

impl Writer {
    pub fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    pub fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_be_bytes());
    }
    pub fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_be_bytes());
    }
    pub fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_be_bytes());
    }
    pub fn bool(&mut self, v: bool) {
        self.u8(v as u8);
    }
    pub fn f32(&mut self, v: f32) {
        self.u32(v.to_bits());
    }
    /// Callers keep strings within the limits checked by `Reader::str`.
    pub fn str(&mut self, s: &str) {
        self.u16(s.len() as u16);
        self.0.extend_from_slice(s.as_bytes());
    }
}

pub(crate) struct Reader<'a> {
    buf: &'a [u8],
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Reader { buf }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        if self.buf.len() < n {
            return Err(DecodeError::Truncated);
        }
        let (head, rest) = self.buf.split_at(n);
        self.buf = rest;
        Ok(head)
    }
    pub fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take(1)?[0])
    }
    pub fn u16(&mut self) -> Result<u16, DecodeError> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().unwrap()))
    }
    pub fn u32(&mut self) -> Result<u32, DecodeError> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    pub fn u64(&mut self) -> Result<u64, DecodeError> {
        Ok(u64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }
    pub fn bool(&mut self) -> Result<bool, DecodeError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(DecodeError::Invalid("bool")),
        }
    }
    pub fn f32(&mut self) -> Result<f32, DecodeError> {
        let v = f32::from_bits(self.u32()?);
        if v.is_finite() {
            Ok(v)
        } else {
            Err(DecodeError::Invalid("float"))
        }
    }
    pub fn str(&mut self, max_bytes: usize) -> Result<String, DecodeError> {
        let n = self.u16()? as usize;
        if n > max_bytes {
            return Err(DecodeError::Invalid("string length"));
        }
        let bytes = self.take(n)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| DecodeError::Invalid("utf-8"))
    }
    pub fn rest(&mut self) -> &'a [u8] {
        std::mem::take(&mut self.buf)
    }
    pub fn finish(&self) -> Result<(), DecodeError> {
        if self.buf.is_empty() {
            Ok(())
        } else {
            Err(DecodeError::TrailingBytes)
        }
    }
}

/// Wraps a message body with its `u32` length prefix.
pub(crate) fn frame(body: impl FnOnce(&mut Writer)) -> Vec<u8> {
    let mut w = Writer(vec![0; 4]);
    body(&mut w);
    let len = (w.0.len() - 4) as u32;
    w.0[..4].copy_from_slice(&len.to_be_bytes());
    w.0
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
        let mut r = Reader::new(datagram);
        if r.u8()? != DATAGRAM_VERSION {
            return Err(DecodeError::Invalid("datagram version"));
        }
        if r.u8()? != KIND_SNAPSHOT {
            return Err(DecodeError::Invalid("datagram kind"));
        }
        let (tick, room_revision, index, count) = (r.u64()?, r.u64()?, r.u8()?, r.u8()?);
        if count == 0 || count as usize > MAX_SNAPSHOT_FRAGMENTS || index >= count {
            return Err(DecodeError::Invalid("fragment index"));
        }
        Ok(SnapshotFragment {
            tick,
            room_revision,
            index,
            count,
            payload: r.rest().to_vec(),
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
                let mut w = Writer::default();
                w.u8(DATAGRAM_VERSION);
                w.u8(KIND_SNAPSHOT);
                w.u64(tick);
                w.u64(room_revision);
                w.u8(i as u8);
                w.u8(count as u8);
                w.0.extend_from_slice(part);
                w.0
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
        let a = frame(|w| w.u8(1));
        let b = frame(|w| w.str("hello"));
        let joined: Vec<u8> = a.iter().chain(&b).copied().collect();
        let mut d = FrameDecoder::default();
        for byte in &joined[..3] {
            d.push(&[*byte]);
            assert_eq!(d.next_frame(), Ok(None));
        }
        d.push(&joined[3..]);
        assert_eq!(d.next_frame(), Ok(Some(vec![1])));
        assert_eq!(d.next_frame().unwrap().unwrap().len(), 7);
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
