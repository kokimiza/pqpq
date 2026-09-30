//! Native QUIC adapter on quiche. quiche is sans-I/O, so this module owns the
//! UDP socket, timers and pacing for every native connection.

use std::collections::HashMap;
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use pqpq_protocol::{FrameDecoder, NATIVE_ALPN};
use tokio::net::UdpSocket;
use tokio::sync::{Notify, watch};

use super::{CLOSE_LINGER, ConnectionHandle, EventSink, MAX_RELIABLE_BACKLOG};
use crate::config::Config;

const MAX_UDP_PAYLOAD: usize = 1350;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
const IDLE_TIMEOUT_MS: u64 = 10_000;
/// Application close codes.
const CLOSE_NORMAL: u64 = 0;
const CLOSE_PROTOCOL: u64 = 1;
const CLOSE_BUSY: u64 = 2;
const CONTROL_STREAM: u64 = 0;

struct Client {
    conn: quiche::Connection,
    handle: ConnectionHandle,
    decoder: FrameDecoder,
    /// Control bytes quiche has not accepted yet (flow control).
    unsent: Vec<u8>,
    announced: bool,
    created: Instant,
    close_at: Option<Instant>,
    /// Packet held back until quiche's pacing time.
    held: Option<(Vec<u8>, SocketAddr, Instant)>,
}

pub struct Native {
    io: Io,
    config: quiche::Config,
}

/// Everything the packet loop needs besides the quiche config.
struct Io {
    socket: UdpSocket,
    /// Same socket, used for sends: a plain non-blocking syscall that does not
    /// depend on tokio's cached write readiness (unreliable on Windows).
    sender: std::net::UdpSocket,
    local: SocketAddr,
    cid_key: RandomState,
}

impl Native {
    pub async fn bind(cfg: &Config) -> Result<Native, String> {
        // Rust handles Unicode paths on Windows; BoringSSL's file API uses
        // narrow C paths and cannot open certificates in Japanese folders.
        let tls_error = |e| format!("native TLS: {e}");
        let cert_pem =
            std::fs::read(&cfg.tls_cert).map_err(|e| format!("native TLS certificate: {e}"))?;
        let key_pem = std::fs::read(&cfg.tls_key).map_err(|e| format!("native TLS key: {e}"))?;
        let mut chain = boring::x509::X509::stack_from_pem(&cert_pem)
            .map_err(tls_error)?
            .into_iter();
        let leaf = chain
            .next()
            .ok_or("native TLS certificate chain is empty")?;
        let key = boring::pkey::PKey::private_key_from_pem(&key_pem).map_err(tls_error)?;
        let mut tls = boring::ssl::SslContextBuilder::new(boring::ssl::SslMethod::tls())
            .map_err(tls_error)?;
        tls.set_certificate(&leaf).map_err(tls_error)?;
        for cert in chain {
            tls.add_extra_chain_cert(cert).map_err(tls_error)?;
        }
        tls.set_private_key(&key).map_err(tls_error)?;
        tls.check_private_key().map_err(tls_error)?;
        let mut config = quiche::Config::with_boring_ssl_ctx_builder(quiche::PROTOCOL_VERSION, tls)
            .map_err(|e| e.to_string())?;
        config
            .set_application_protos(&[NATIVE_ALPN])
            .map_err(|e| e.to_string())?;
        config.set_max_idle_timeout(IDLE_TIMEOUT_MS);
        config.set_max_recv_udp_payload_size(MAX_UDP_PAYLOAD);
        config.set_max_send_udp_payload_size(MAX_UDP_PAYLOAD);
        config.set_initial_max_data(1_000_000);
        config.set_initial_max_stream_data_bidi_remote(MAX_RELIABLE_BACKLOG as u64);
        config.set_initial_max_stream_data_bidi_local(MAX_RELIABLE_BACKLOG as u64);
        // Only the client-opened control stream 0 is allowed.
        config.set_initial_max_streams_bidi(1);
        config.set_initial_max_streams_uni(0);
        config.set_disable_active_migration(true);
        config.enable_dgram(true, 64, 64);
        let bind_error = |e: std::io::Error| format!("native UDP {}: {e}", cfg.native_addr);
        let socket = std::net::UdpSocket::bind(cfg.native_addr).map_err(bind_error)?;
        socket.set_nonblocking(true).map_err(bind_error)?;
        let sender = socket.try_clone().map_err(bind_error)?;
        let socket = UdpSocket::from_std(socket).map_err(bind_error)?;
        let local = socket.local_addr().map_err(|e| e.to_string())?;
        Ok(Native {
            io: Io {
                socket,
                sender,
                local,
                cid_key: RandomState::new(),
            },
            config,
        })
    }

    pub async fn run(self, sink: EventSink, mut shutdown: watch::Receiver<bool>) {
        let Native { io, mut config } = self;
        let wake = Arc::new(Notify::new());
        let mut clients: HashMap<quiche::ConnectionId<'static>, Client> = HashMap::new();
        let mut buf = vec![0; 65535];
        let mut out = vec![0; MAX_UDP_PAYLOAD];
        loop {
            let deadline = clients.values().filter_map(next_deadline).min();
            let sleep = tokio::time::sleep_until(
                deadline
                    .unwrap_or_else(|| Instant::now() + Duration::from_secs(60))
                    .into(),
            );
            tokio::select! {
                r = io.socket.recv_from(&mut buf) => {
                    let mut next = r.map(Some);
                    // A busy socket must yield to timers and other clients.
                    for _ in 0..64 {
                        match next {
                            Ok(Some((len, from))) => io.on_packet(&mut config, &mut clients, &mut buf[..len], from, &mut out, &wake, &sink),
                            Ok(None) => break,
                            // Windows reports ICMP port-unreachable as a reset; not fatal.
                            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
                            Err(e) => {
                                if e.kind() != io::ErrorKind::ConnectionReset {
                                    eprintln!("native: recv error: {}", e.kind());
                                }
                                break;
                            }
                        }
                        next = io.socket.try_recv_from(&mut buf).map(Some);
                    }
                }
                _ = sleep => {}
                _ = wake.notified() => {}
                _ = shutdown.changed() => {
                    for c in clients.values_mut() {
                        let _ = c.conn.close(true, CLOSE_NORMAL, b"server shutdown");
                        io.flush(c, &mut out, Instant::now());
                    }
                    return;
                }
            }
            let now = Instant::now();
            clients.retain(|_, c| {
                io.service(c, &mut buf, &mut out, &sink, now);
                if c.conn.is_closed() {
                    sink.disconnected(&c.handle);
                    sink.release();
                    return false;
                }
                true
            });
        }
    }
}

impl Io {
    #[allow(clippy::too_many_arguments)]
    fn on_packet(
        &self,
        config: &mut quiche::Config,
        clients: &mut HashMap<quiche::ConnectionId<'static>, Client>,
        pkt: &mut [u8],
        from: SocketAddr,
        out: &mut [u8],
        wake: &Arc<Notify>,
        sink: &EventSink,
    ) {
        if pkt.len() > MAX_UDP_PAYLOAD {
            return;
        }
        let Ok(hdr) = quiche::Header::from_slice(pkt, quiche::MAX_CONN_ID_LEN) else {
            return;
        };
        let dcid = hdr.dcid.clone().into_owned();
        let derived = self.derive_cid(&hdr.dcid);
        let key = if clients.contains_key(&dcid) {
            dcid
        } else if clients.contains_key(&derived) {
            derived
        } else {
            if hdr.ty != quiche::Type::Initial {
                return;
            }
            if !quiche::version_is_supported(hdr.version) {
                if let Ok(n) = quiche::negotiate_version(&hdr.scid, &hdr.dcid, out) {
                    let _ = self.sender.send_to(&out[..n], from);
                }
                return;
            }
            // ponytail: no Retry address validation yet; add it if spoofed floods matter.
            if !sink.admit() {
                return;
            }
            let conn = match quiche::accept(&derived, None, self.local, from, config) {
                Ok(c) => c,
                Err(_) => {
                    sink.release();
                    return;
                },
            };
            clients.insert(
                derived.clone(),
                Client {
                    conn,
                    handle: ConnectionHandle::new(wake.clone()),
                    decoder: FrameDecoder::default(),
                    unsent: Vec::new(),
                    announced: false,
                    created: Instant::now(),
                    close_at: None,
                    held: None,
                },
            );
            derived
        };
        if let Some(c) = clients.get_mut(&key) {
            let _ = c.conn.recv(
                pkt,
                quiche::RecvInfo {
                    from,
                    to: self.local,
                },
            );
        }
    }

    /// Stable server connection id for a client's initial destination id, so
    /// retransmitted Initial packets reach the same connection.
    /// Always MAX_CONN_ID_LEN long: short headers carry no length, and
    /// `Header::from_slice` assumes this one.
    fn derive_cid(&self, dcid: &[u8]) -> quiche::ConnectionId<'static> {
        let mut id = Vec::with_capacity(24);
        for salt in 0u8..3 {
            let mut h = self.cid_key.build_hasher();
            h.write_u8(salt);
            h.write(dcid);
            id.extend_from_slice(&h.finish().to_be_bytes());
        }
        id.truncate(quiche::MAX_CONN_ID_LEN);
        quiche::ConnectionId::from_vec(id)
    }

    fn service(
        &self,
        c: &mut Client,
        buf: &mut [u8],
        out: &mut [u8],
        sink: &EventSink,
        now: Instant,
    ) {
        if c.conn.timeout_instant().is_some_and(|t| t <= now) {
            c.conn.on_timeout();
        }
        if !c.conn.is_established() && now - c.created > HANDSHAKE_TIMEOUT {
            let _ = c.conn.close(false, 0x1, b"handshake timeout");
        }
        if c.conn.is_established() && !c.conn.is_closed() {
            self.exchange(c, buf, sink, now);
        }
        if c.close_at.is_some_and(|t| t <= now) {
            let _ = c.conn.close(true, CLOSE_NORMAL, b"closed");
        }
        self.flush(c, out, now);
    }

    /// Moves data between quiche and the connection handle.
    fn exchange(&self, c: &mut Client, buf: &mut [u8], sink: &EventSink, now: Instant) {
        if !c.announced {
            c.announced = true;
            if c.conn.dgram_max_writable_len().is_none() {
                let _ = c.conn.close(true, CLOSE_PROTOCOL, b"datagram unsupported");
                return;
            }
            if !sink.connected(c.handle.clone()) {
                let _ = c.conn.close(true, CLOSE_BUSY, b"busy");
                return;
            }
        }
        for stream in c.conn.readable() {
            while let Ok((n, _fin)) = c.conn.stream_recv(stream, buf) {
                if stream != CONTROL_STREAM {
                    let _ = c.conn.close(true, CLOSE_PROTOCOL, b"unexpected stream");
                    return;
                }
                c.decoder.push(&buf[..n]);
            }
        }
        loop {
            match c.decoder.next_frame() {
                Ok(Some(body)) => {
                    if !c.handle.allow_control(now) || !sink.reliable(c.handle.id(), body) {
                        let _ = c.conn.close(true, CLOSE_PROTOCOL, b"too many messages");
                        return;
                    }
                },
                Ok(None) => break,
                Err(_) => {
                    let _ = c.conn.close(true, CLOSE_PROTOCOL, b"bad frame");
                    return;
                },
            }
        }
        while let Ok(n) = c.conn.dgram_recv(buf) {
            if !c.handle.push_input(&buf[..n], now) {
                let _ = c.conn.close(true, CLOSE_PROTOCOL, b"too many datagrams");
                return;
            }
        }

        let outgoing = c.handle.take_outgoing();
        c.unsent.extend_from_slice(&outgoing.reliable);
        if c.unsent.len() > MAX_RELIABLE_BACKLOG {
            let _ = c.conn.close(true, CLOSE_BUSY, b"receiver too slow");
            return;
        }
        while !c.unsent.is_empty() {
            match c.conn.stream_send(CONTROL_STREAM, &c.unsent, false) {
                Ok(n) if n > 0 => {
                    c.unsent.drain(..n);
                },
                _ => break,
            }
        }
        if let Some(n) = c.conn.dgram_max_writable_len() {
            c.handle.set_max_datagram(n);
        }
        if !outgoing.datagrams.is_empty() {
            // Older snapshots still queued are worthless now.
            c.conn.dgram_purge_outgoing(|_| true);
            for d in &outgoing.datagrams {
                let _ = c.conn.dgram_send(d);
            }
        }
        if outgoing.close && c.close_at.is_none() {
            c.close_at = Some(now + CLOSE_LINGER);
        }
    }

    /// Writes packets, holding one back when quiche asks for pacing.
    fn flush(&self, c: &mut Client, out: &mut [u8], now: Instant) {
        loop {
            if let Some((pkt, to, at)) = &c.held {
                if *at > now {
                    return;
                }
                let _ = self.sender.send_to(pkt, *to);
                c.held = None;
            }
            match c.conn.send(out) {
                Ok((n, info)) if info.at > now + Duration::from_millis(1) => {
                    c.held = Some((out[..n].to_vec(), info.to, info.at));
                },
                // A full socket buffer drops the packet; QUIC recovers it.
                Ok((n, info)) => {
                    let _ = self.sender.send_to(&out[..n], info.to);
                },
                Err(quiche::Error::Done) => return,
                Err(_) => {
                    let _ = c.conn.close(false, 0x1, b"send failed");
                    return;
                },
            }
        }
    }
}

fn next_deadline(c: &Client) -> Option<Instant> {
    if c.conn.is_draining() {
        return c.conn.timeout_instant();
    }
    [
        c.conn.timeout_instant(),
        c.held.as_ref().map(|h| h.2),
        c.close_at,
        (!c.conn.is_established()).then_some(c.created + HANDSHAKE_TIMEOUT),
    ]
    .into_iter()
    .flatten()
    .min()
}
