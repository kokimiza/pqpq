//! WebTransport over HTTP/3 adapter (wtransport). The browser opens one
//! bidirectional stream for control messages and uses session datagrams.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use pqpq_protocol::{FrameDecoder, WEBTRANSPORT_PATH};
use tokio::sync::{Notify, watch};
use wtransport::endpoint::IncomingSession;
use wtransport::{Endpoint, Identity, ServerConfig, VarInt};

use super::{CLOSE_LINGER, ConnectionHandle, EventSink};
use crate::config::Config;

const SETUP_TIMEOUT: Duration = Duration::from_secs(5);

pub struct Web {
    endpoints: Vec<Endpoint<wtransport::endpoint::endpoint_side::Server>>,
    origin: Arc<str>,
}

impl Web {
    pub async fn bind(cfg: &Config) -> Result<Web, String> {
        let identity = Identity::load_pemfiles(&cfg.tls_cert, &cfg.tls_key)
            .await
            .map_err(|e| format!("WebTransport TLS: {e}"))?;
        let endpoint = |addr: SocketAddr| {
            let config = ServerConfig::builder()
                .with_bind_address(addr)
                .with_identity(identity.clone_identity())
                .keep_alive_interval(Some(Duration::from_secs(3)))
                .max_idle_timeout(Some(Duration::from_secs(10)))
                .map_err(|e| e.to_string())?
                .build();
            Endpoint::server(config).map_err(|e| format!("WebTransport UDP {addr}: {e}"))
        };
        let mut endpoints = vec![endpoint(cfg.webtransport_addr)?];
        // Browsers may resolve "localhost" to ::1 first; QUIC has no fallback
        // like TCP happy eyeballs, so listen on both loopbacks (best effort).
        if cfg.webtransport_addr.ip() == IpAddr::V4(Ipv4Addr::LOCALHOST) {
            let v6 = SocketAddr::new(
                IpAddr::V6(Ipv6Addr::LOCALHOST),
                cfg.webtransport_addr.port(),
            );
            endpoints.extend(endpoint(v6).ok());
        }
        Ok(Web {
            endpoints,
            origin: cfg.web_origin.as_str().into(),
        })
    }

    pub async fn run(self, sink: EventSink, shutdown: watch::Receiver<bool>) {
        let loops: Vec<_> = self
            .endpoints
            .into_iter()
            .map(|ep| {
                tokio::spawn(accept(
                    ep,
                    sink.clone(),
                    self.origin.clone(),
                    shutdown.clone(),
                ))
            })
            .collect();
        for l in loops {
            let _ = l.await;
        }
    }
}

async fn accept(
    endpoint: Endpoint<wtransport::endpoint::endpoint_side::Server>,
    sink: EventSink,
    origin: Arc<str>,
    mut shutdown: watch::Receiver<bool>,
) {
    loop {
        tokio::select! {
            incoming = endpoint.accept() => {
                if !sink.admit() {
                    incoming.refuse();
                    continue;
                }
                let (sink, origin, shutdown) = (sink.clone(), origin.clone(), shutdown.clone());
                tokio::spawn(async move {
                    session(incoming, &sink, &origin, shutdown).await;
                    sink.release();
                });
            }
            _ = shutdown.changed() => {
                endpoint.close(VarInt::from_u32(0), b"server shutdown");
                return;
            }
        }
    }
}

async fn session(
    incoming: IncomingSession,
    sink: &EventSink,
    origin: &str,
    mut shutdown: watch::Receiver<bool>,
) {
    let Ok(Ok(request)) = tokio::time::timeout(SETUP_TIMEOUT, incoming).await else {
        return;
    };
    if request.path() != WEBTRANSPORT_PATH {
        request.not_found().await;
        return;
    }
    // Exact match only; this limits which page may connect, not who plays.
    if request.origin() != Some(origin) {
        request.forbidden().await;
        return;
    }
    let Ok(conn) = request.accept().await else {
        return;
    };
    let Some(max_datagram) = conn.max_datagram_size() else {
        conn.close(VarInt::from_u32(1), b"datagram unsupported");
        return;
    };
    let Ok(Ok((mut send, mut recv))) = tokio::time::timeout(SETUP_TIMEOUT, conn.accept_bi()).await
    else {
        conn.close(VarInt::from_u32(1), b"no control stream");
        return;
    };

    let wake = Arc::new(Notify::new());
    let handle = ConnectionHandle::new(wake.clone());
    handle.set_max_datagram(max_datagram);
    if !sink.connected(handle.clone()) {
        conn.close(VarInt::from_u32(2), b"busy");
        return;
    }
    let mut decoder = FrameDecoder::default();
    let mut buf = vec![0; 16 * 1024];
    let close_code = 'io: loop {
        tokio::select! {
            read = recv.read(&mut buf) => {
                let Ok(Some(n)) = read else { break 'io 0 };
                decoder.push(&buf[..n]);
                loop {
                    match decoder.next_frame() {
                        Ok(Some(body)) => {
                            if !handle.allow_control(Instant::now()) || !sink.reliable(handle.id(), body) {
                                break 'io 1;
                            }
                        }
                        Ok(None) => break,
                        Err(_) => break 'io 1,
                    }
                }
            }
            datagram = conn.receive_datagram() => {
                let Ok(d) = datagram else { break 'io 0 };
                if !handle.push_input(&d.payload(), Instant::now()) {
                    break 'io 1;
                }
            }
            _ = wake.notified() => {
                let out = handle.take_outgoing();
                if !out.reliable.is_empty() {
                    // Flow control from a stalled reader must not trap this
                    // session forever or prevent shutdown from releasing it.
                    let sent = tokio::select! {
                        result = tokio::time::timeout(Duration::from_secs(2), send.write_all(&out.reliable)) => {
                            matches!(result, Ok(Ok(())))
                        }
                        _ = shutdown.changed() => false,
                    };
                    if !sent { break 'io 0; }
                }
                if let Some(n) = conn.max_datagram_size() {
                    handle.set_max_datagram(n);
                }
                for d in out.datagrams {
                    // Too large or congested: dropped, the next snapshot replaces it.
                    let _ = conn.send_datagram(d);
                }
                if out.close {
                    tokio::time::sleep(CLOSE_LINGER).await;
                    break 'io 0;
                }
            }
            _ = shutdown.changed() => break 'io 0,
        }
    };
    conn.close(VarInt::from_u32(close_code), b"closed");
    sink.disconnected(&handle);
}
