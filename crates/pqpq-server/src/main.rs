//! pqpq game server: Native QUIC, WebTransport and the static web client in one
//! process. All game state lives in memory and is lost on exit by design.

mod assets;
mod config;
mod race;
mod rooms;
mod simulation;
mod transport;

use std::process::ExitCode;
use std::time::Duration;

use tokio::sync::watch;

use crate::assets::Assets;
use crate::config::Config;
use crate::transport::{CLOSE_LINGER, native::Native, web::Web};

/// Upper bound of a graceful stop.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(3);

#[tokio::main]
async fn main() -> ExitCode {
    match serve().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("pqpq-server: {e}");
            ExitCode::from(1)
        },
    }
}

async fn serve() -> Result<(), String> {
    let cfg = Config::from_env()?;
    // Bind everything first: a busy port or bad certificate stops startup.
    let native = Native::bind(&cfg).await?;
    let web = Web::bind(&cfg).await?;
    let assets = Assets::bind(&cfg).await?;

    let (sink, source) = transport::event_queue(cfg.max_connections);
    let (stop_io, io_stopped) = watch::channel(false);
    let (stop_game, game_stopped) = watch::channel(false);
    let io = [
        tokio::spawn(native.run(sink.clone(), io_stopped.clone())),
        tokio::spawn(web.run(sink, io_stopped.clone())),
        tokio::spawn(assets.run(io_stopped)),
    ];
    eprintln!(
        "pqpq-server: native udp {} / webtransport udp {} / https tcp {}",
        cfg.native_addr, cfg.webtransport_addr, cfg.https_addr
    );
    let game = tokio::spawn(simulation::run(cfg, source, game_stopped));

    shutdown_signal().await;
    eprintln!("pqpq-server: stopping");
    // The game tells every client why, then the transports get a moment to
    // deliver it before closing. Nothing is saved.
    let _ = stop_game.send(true);
    let _ = tokio::time::timeout(SHUTDOWN_GRACE / 2, game).await;
    tokio::time::sleep(CLOSE_LINGER * 2).await;
    let _ = stop_io.send(true);
    let _ = tokio::time::timeout(SHUTDOWN_GRACE / 4, futures_join(io)).await;
    Ok(())
}

async fn futures_join(tasks: [tokio::task::JoinHandle<()>; 3]) {
    for t in tasks {
        let _ = t.await;
    }
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    let _ = tokio::signal::ctrl_c().await;
}
