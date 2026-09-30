//! Native QUIC client on quiche: control stream 0 plus DATAGRAM snapshots.
//! Runs as one task that owns the UDP socket and all quiche timers.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use pqpq_protocol::{
    ClientMessage, FrameDecoder, GameSnapshot, InputDatagram, NATIVE_ALPN, ServerMessage,
    SnapshotAssembler, SnapshotFragment,
};
use tokio::net::UdpSocket;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::config::Config;

const MAX_UDP_PAYLOAD: usize = 1350;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const CONTROL_STREAM: u64 = 0;
/// How long a clean close may take before the task just ends.
const CLOSE_WAIT: Duration = Duration::from_millis(300);

pub enum NetEvent {
    Connected,
    Message(ServerMessage),
    Snapshot(GameSnapshot),
    Closed(String),
}

pub enum NetCommand {
    Send(ClientMessage),
    Input(InputDatagram),
    Close,
}

pub async fn run(
    cfg: Config,
    commands: UnboundedReceiver<NetCommand>,
    events: UnboundedSender<NetEvent>,
) {
    let reason = match session(&cfg, commands, &events).await {
        Ok(()) => "接続を終了しました".to_owned(),
        Err(e) => e,
    };
    let _ = events.send(NetEvent::Closed(reason));
}

async fn session(
    cfg: &Config,
    mut commands: UnboundedReceiver<NetCommand>,
    events: &UnboundedSender<NetEvent>,
) -> Result<(), String> {
    let unreachable =
        |e: std::io::Error| format!("接続先 {} に到達できません: {e}", cfg.server_addr);
    // Prefer IPv4: "localhost" may list ::1 first while the server listens on
    // 127.0.0.1, and QUIC has no TCP-style fallback between addresses.
    let addrs: Vec<SocketAddr> = tokio::net::lookup_host(&cfg.server_addr)
        .await
        .map_err(unreachable)?
        .collect();
    let peer = *addrs
        .iter()
        .find(|a| a.is_ipv4())
        .or(addrs.first())
        .ok_or_else(|| format!("接続先 {} を解決できません", cfg.server_addr))?;
    let bind: SocketAddr = if peer.is_ipv4() {
        "0.0.0.0:0"
    } else {
        "[::]:0"
    }
    .parse()
    .unwrap();
    let socket = std::net::UdpSocket::bind(bind).map_err(unreachable)?;
    socket.set_nonblocking(true).map_err(unreachable)?;
    // Sends go through a plain clone: tokio's cached write readiness is not
    // reliable for UDP on Windows, and a dropped packet is recovered by QUIC.
    let sender = socket.try_clone().map_err(unreachable)?;
    let socket = UdpSocket::from_std(socket).map_err(unreachable)?;
    let local = socket.local_addr().map_err(unreachable)?;

    let mut config = if let Some(ca) = &cfg.ca_file {
        // Read through Rust to support Unicode paths on Windows too.
        let pem = std::fs::read(ca).map_err(|e| format!("CAファイル {}: {e}", ca.display()))?;
        let tls_error = |e| format!("CAファイル {}: {e}", ca.display());
        let certs = boring::x509::X509::stack_from_pem(&pem).map_err(tls_error)?;
        if certs.is_empty() {
            return Err(format!("CAファイル {} に証明書がありません", ca.display()));
        }
        let mut tls = boring::ssl::SslContextBuilder::new(boring::ssl::SslMethod::tls())
            .map_err(tls_error)?;
        for cert in certs {
            tls.cert_store_mut().add_cert(cert).map_err(tls_error)?;
        }
        quiche::Config::with_boring_ssl_ctx_builder(quiche::PROTOCOL_VERSION, tls)
    } else {
        quiche::Config::new(quiche::PROTOCOL_VERSION)
    }
    .map_err(|e| e.to_string())?;
    // Always verify the server; development uses a local CA, never "insecure".
    config.verify_peer(true);
    config
        .set_application_protos(&[NATIVE_ALPN])
        .map_err(|e| e.to_string())?;
    config.set_max_idle_timeout(10_000);
    config.set_max_recv_udp_payload_size(MAX_UDP_PAYLOAD);
    config.set_max_send_udp_payload_size(MAX_UDP_PAYLOAD);
    config.set_initial_max_data(1_000_000);
    config.set_initial_max_stream_data_bidi_local(256 * 1024);
    config.set_initial_max_stream_data_bidi_remote(256 * 1024);
    config.set_initial_max_streams_bidi(0);
    config.set_initial_max_streams_uni(0);
    config.set_disable_active_migration(true);
    config.enable_dgram(true, 64, 64);

    let scid = random_cid();
    let scid = quiche::ConnectionId::from_ref(&scid);
    let mut conn = quiche::connect(Some(&cfg.server_name), &scid, local, peer, &mut config)
        .map_err(|e| format!("接続を開始できません: {e}"))?;

    let started = Instant::now();
    let mut buf = vec![0u8; 65535];
    let mut out = vec![0u8; MAX_UDP_PAYLOAD];
    let mut decoder = FrameDecoder::default();
    let mut assembler = SnapshotAssembler::default();
    let mut unsent: Vec<u8> = Vec::new();
    let mut held: Option<(Vec<u8>, SocketAddr, Instant)> = None;
    let mut announced = false;
    let mut closing: Option<Instant> = None;

    loop {
        flush(&sender, &mut conn, &mut out, &mut held);
        if conn.is_closed() {
            return closed_reason(&conn, announced);
        }
        let deadline = [
            conn.timeout_instant(),
            held.as_ref().map(|h| h.2),
            (!announced).then_some(started + CONNECT_TIMEOUT),
            closing,
        ]
        .into_iter()
        .flatten()
        .min()
        .unwrap_or_else(|| Instant::now() + Duration::from_secs(60));

        tokio::select! {
            r = socket.recv_from(&mut buf) => {
                let mut next = r;
                // Windows reports ICMP port-unreachable as an error; keep going.
                for _ in 0..64 {
                    let Ok((len, from)) = next else { break };
                    let _ = conn.recv(&mut buf[..len], quiche::RecvInfo { from, to: local });
                    next = socket.try_recv_from(&mut buf);
                }
            }
            _ = tokio::time::sleep_until(deadline.into()) => {}
            command = commands.recv(), if closing.is_none() => match command {
                Some(NetCommand::Send(msg)) => unsent.extend_from_slice(&msg.encode_frame()),
                Some(NetCommand::Input(input)) => {
                    let _ = conn.dgram_send(&input.encode());
                }
                Some(NetCommand::Close) | None => {
                    closing = Some(Instant::now() + CLOSE_WAIT);
                }
            },
        }

        let now = Instant::now();
        if conn.timeout_instant().is_some_and(|t| t <= now) {
            conn.on_timeout();
        }
        if !announced && now - started > CONNECT_TIMEOUT {
            return Err(format!("接続がタイムアウトしました（{}）", cfg.server_addr));
        }
        if unsent.len() > 256 * 1024 {
            return Err("サーバへの送信が滞っています".into());
        }
        if closing.is_some_and(|t| t <= now) {
            return Ok(());
        }
        if !conn.is_established() {
            continue;
        }
        if !announced {
            announced = true;
            if conn.dgram_max_writable_len().is_none() {
                return Err("サーバがDatagramに対応していません".into());
            }
            let _ = events.send(NetEvent::Connected);
        }

        while let Ok((n, _)) = conn.stream_recv(CONTROL_STREAM, &mut buf) {
            decoder.push(&buf[..n]);
        }
        loop {
            match decoder.next_frame() {
                Ok(Some(body)) => match ServerMessage::decode(&body) {
                    Ok(msg) => {
                        let _ = events.send(NetEvent::Message(msg));
                    },
                    Err(_) => {
                        return Err("通信形式が一致しません（サーバの版を確認してください）".into());
                    },
                },
                Ok(None) => break,
                Err(_) => return Err("通信形式が一致しません".into()),
            }
        }
        let now_ms = now.duration_since(started).as_secs_f64() * 1000.0;
        while let Ok(n) = conn.dgram_recv(&mut buf) {
            let Ok(fragment) = SnapshotFragment::decode(&buf[..n]) else {
                continue;
            };
            if let Some(body) = assembler.push(fragment, now_ms)
                && let Ok(snapshot) = GameSnapshot::decode(&body)
            {
                let _ = events.send(NetEvent::Snapshot(snapshot));
            }
        }

        while !unsent.is_empty() {
            match conn.stream_send(CONTROL_STREAM, &unsent, false) {
                Ok(n) if n > 0 => {
                    unsent.drain(..n);
                },
                _ => break,
            }
        }
        if closing.is_some() && unsent.is_empty() && !conn.is_draining() {
            let _ = conn.close(true, 0, b"bye");
        }
    }
}

fn flush(
    socket: &std::net::UdpSocket,
    conn: &mut quiche::Connection,
    out: &mut [u8],
    held: &mut Option<(Vec<u8>, SocketAddr, Instant)>,
) {
    let now = Instant::now();
    loop {
        if let Some((pkt, to, at)) = held {
            if *at > now {
                return;
            }
            let _ = socket.send_to(pkt, *to);
            *held = None;
        }
        match conn.send(out) {
            Ok((n, info)) if info.at > now + Duration::from_millis(1) => {
                *held = Some((out[..n].to_vec(), info.to, info.at));
            },
            Ok((n, info)) => {
                let _ = socket.send_to(&out[..n], info.to);
            },
            Err(_) => return,
        }
    }
}

fn closed_reason(conn: &quiche::Connection, established: bool) -> Result<(), String> {
    if !established {
        return Err("接続できませんでした。接続先または証明書（PQPQ_CA_FILE / PQPQ_TLS_SERVER_NAME）を確認してください".into());
    }
    if conn.is_timed_out() {
        return Err("サーバから応答がありません".into());
    }
    match conn.peer_error() {
        Some(e) if e.is_app && e.error_code == 0 => Err("サーバが接続を終了しました".into()),
        Some(e) => Err(format!(
            "サーバが接続を終了しました（{}）",
            String::from_utf8_lossy(&e.reason)
        )),
        None => Ok(()),
    }
}

fn random_cid() -> [u8; 16] {
    let mut id = [0u8; 16];
    for (i, chunk) in id.chunks_mut(8).enumerate() {
        let mut h = RandomState::new().build_hasher();
        h.write_usize(i);
        h.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
        );
        chunk.copy_from_slice(&h.finish().to_be_bytes());
    }
    id
}
