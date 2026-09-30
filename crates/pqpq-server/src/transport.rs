//! Common connection model for Native QUIC and WebTransport. The game loop
//! only sees `ConnectionHandle` and `Event`, never the transport type.

pub mod native;
pub mod web;

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use tokio::sync::{Notify, mpsc};

/// Unsent control bytes allowed per connection before it is dropped.
pub const MAX_RELIABLE_BACKLOG: usize = 256 * 1024;
/// Non-disconnect events queued towards the game loop.
const MAX_QUEUED_EVENTS: usize = 1024;
/// Input datagrams kept between two ticks; more is discarded.
const MAX_INPUTS_PER_TICK: usize = 8;
/// Connections still in handshake or before Join, on top of the player cap.
pub const PENDING_ALLOWANCE: usize = 32;
/// Time given to deliver a final Error message before closing.
pub const CLOSE_LINGER: std::time::Duration = std::time::Duration::from_millis(250);

pub enum Event {
    Connected(ConnectionHandle),
    /// One control-stream message body.
    Reliable(u64, Vec<u8>),
    Disconnected(u64),
}

/// Transport side of the event queue. Disconnects are always accepted so the
/// game never keeps a dead player; other events are refused when saturated.
#[derive(Clone)]
pub struct EventSink {
    tx: mpsc::UnboundedSender<Event>,
    queued: Arc<AtomicUsize>,
    connections: Arc<AtomicUsize>,
    connection_limit: usize,
}

pub struct EventSource {
    rx: mpsc::UnboundedReceiver<Event>,
    queued: Arc<AtomicUsize>,
}

pub fn event_queue(max_connections: usize) -> (EventSink, EventSource) {
    let (tx, rx) = mpsc::unbounded_channel();
    let queued = Arc::new(AtomicUsize::new(0));
    let sink = EventSink {
        tx,
        queued: queued.clone(),
        connections: Arc::default(),
        connection_limit: max_connections + PENDING_ALLOWANCE,
    };
    (sink, EventSource { rx, queued })
}

impl EventSink {
    fn try_send(&self, event: Event) -> bool {
        if self.queued.fetch_add(1, Ordering::AcqRel) >= MAX_QUEUED_EVENTS {
            self.queued.fetch_sub(1, Ordering::AcqRel);
            return false;
        }
        if self.tx.send(event).is_err() {
            self.queued.fetch_sub(1, Ordering::AcqRel);
            return false;
        }
        true
    }

    pub fn connected(&self, handle: ConnectionHandle) -> bool {
        self.try_send(Event::Connected(handle))
    }

    pub fn reliable(&self, id: u64, body: Vec<u8>) -> bool {
        self.try_send(Event::Reliable(id, body))
    }

    pub fn disconnected(&self, handle: &ConnectionHandle) {
        handle.0.closed.store(true, Ordering::Release);
        let _ = self.tx.send(Event::Disconnected(handle.id()));
    }

    /// Reserves a transport slot; false when the server is full.
    pub fn admit(&self) -> bool {
        let n = self.connections.fetch_add(1, Ordering::AcqRel);
        if n >= self.connection_limit {
            self.connections.fetch_sub(1, Ordering::AcqRel);
            return false;
        }
        true
    }

    pub fn release(&self) {
        self.connections.fetch_sub(1, Ordering::AcqRel);
    }
}

impl EventSource {
    pub async fn recv(&mut self) -> Option<Event> {
        let event = self.rx.recv().await?;
        if !matches!(event, Event::Disconnected(_)) {
            self.queued.fetch_sub(1, Ordering::AcqRel);
        }
        Some(event)
    }
}

struct TokenBucket {
    tokens: f64,
    capacity: f64,
    per_second: f64,
    last: Instant,
}

impl TokenBucket {
    fn new(per_second: f64, capacity: f64) -> Self {
        TokenBucket {
            tokens: capacity,
            capacity,
            per_second,
            last: Instant::now(),
        }
    }

    fn allow(&mut self, now: Instant) -> bool {
        let elapsed = now.saturating_duration_since(self.last).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.per_second).min(self.capacity);
        self.last = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

struct State {
    reliable: Vec<u8>,
    datagrams: Vec<Vec<u8>>,
    close: bool,
    inputs: Vec<Vec<u8>>,
    /// 60/s sustained, bursts of 120.
    input_rate: TokenBucket,
    /// Excess inputs tolerated before the sender is dropped.
    input_strikes: TokenBucket,
    /// 10/s sustained, bursts of 20.
    control_rate: TokenBucket,
}

struct Shared {
    id: u64,
    wake: Arc<Notify>,
    max_datagram: AtomicUsize,
    closed: AtomicBool,
    state: Mutex<State>,
}

/// One logical connection: a Native QUIC connection or a WebTransport session.
#[derive(Clone)]
pub struct ConnectionHandle(Arc<Shared>);

pub struct Outgoing {
    pub reliable: Vec<u8>,
    pub datagrams: Vec<Vec<u8>>,
    pub close: bool,
}

impl ConnectionHandle {
    pub fn new(wake: Arc<Notify>) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        ConnectionHandle(Arc::new(Shared {
            id: NEXT.fetch_add(1, Ordering::Relaxed),
            wake,
            max_datagram: AtomicUsize::new(0),
            closed: AtomicBool::new(false),
            state: Mutex::new(State {
                reliable: Vec::new(),
                datagrams: Vec::new(),
                close: false,
                inputs: Vec::new(),
                input_rate: TokenBucket::new(60.0, 120.0),
                input_strikes: TokenBucket::new(10.0, 120.0),
                control_rate: TokenBucket::new(10.0, 20.0),
            }),
        }))
    }

    pub fn id(&self) -> u64 {
        self.0.id
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.0.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Ordered control message. A receiver that falls too far behind is closed
    /// instead of making the game wait.
    pub fn send_reliable(&self, frame: &[u8]) {
        let mut st = self.state();
        if st.close {
            return;
        }
        if st.reliable.len() + frame.len() > MAX_RELIABLE_BACKLOG {
            st.close = true;
        } else {
            st.reliable.extend_from_slice(frame);
        }
        drop(st);
        self.0.wake.notify_one();
    }

    /// Replaces any unsent snapshot: only the latest state matters.
    pub fn send_unreliable(&self, datagrams: Vec<Vec<u8>>) {
        self.state().datagrams = datagrams;
        self.0.wake.notify_one();
    }

    /// Closes after already queued control messages are sent.
    pub fn close(&self) {
        self.state().close = true;
        self.0.wake.notify_one();
    }

    /// Game payload limit without transport headers; 0 until known.
    pub fn max_datagram_payload(&self) -> usize {
        self.0.max_datagram.load(Ordering::Relaxed)
    }

    pub fn is_closed(&self) -> bool {
        self.0.closed.load(Ordering::Acquire)
    }

    /// Input datagrams received since the last call.
    pub fn take_inputs(&self) -> Vec<Vec<u8>> {
        std::mem::take(&mut self.state().inputs)
    }

    pub(crate) fn set_max_datagram(&self, n: usize) {
        self.0.max_datagram.store(n, Ordering::Relaxed);
    }

    pub(crate) fn take_outgoing(&self) -> Outgoing {
        let mut st = self.state();
        Outgoing {
            reliable: std::mem::take(&mut st.reliable),
            datagrams: std::mem::take(&mut st.datagrams),
            close: st.close,
        }
    }

    /// False when the peer keeps flooding and must be disconnected.
    pub(crate) fn push_input(&self, datagram: &[u8], now: Instant) -> bool {
        let mut st = self.state();
        if st.input_rate.allow(now) && st.inputs.len() < MAX_INPUTS_PER_TICK {
            st.inputs.push(datagram.to_vec());
            return true;
        }
        st.input_strikes.allow(now)
    }

    /// False when control messages exceed the rate limit.
    pub(crate) fn allow_control(&self, now: Instant) -> bool {
        self.state().control_rate.allow(now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn slow_receiver_is_closed_not_buffered() {
        let h = ConnectionHandle::new(Arc::default());
        h.send_reliable(&vec![0; MAX_RELIABLE_BACKLOG]);
        h.send_reliable(&[0; 1]);
        let out = h.take_outgoing();
        assert!(out.close);
        assert_eq!(out.reliable.len(), MAX_RELIABLE_BACKLOG);
    }

    #[test]
    fn input_flood_is_dropped_then_disconnected() {
        let h = ConnectionHandle::new(Arc::default());
        let now = Instant::now();
        let accepted = (0..300).take_while(|_| h.push_input(&[1], now)).count();
        assert!(accepted < 300);
        assert_eq!(h.take_inputs().len(), MAX_INPUTS_PER_TICK);
        // Sustained 30 Hz is fine forever.
        let h = ConnectionHandle::new(Arc::default());
        for i in 0..3000 {
            assert!(h.push_input(&[1], now + Duration::from_millis(i * 33)));
            h.take_inputs();
        }
    }

    #[test]
    fn event_queue_refuses_when_full_but_keeps_disconnects() {
        let (sink, _source) = event_queue(1);
        let h = ConnectionHandle::new(Arc::default());
        for _ in 0..MAX_QUEUED_EVENTS {
            assert!(sink.reliable(1, vec![]));
        }
        assert!(!sink.reliable(1, vec![]));
        sink.disconnected(&h);
        assert!(h.is_closed());
    }
}
