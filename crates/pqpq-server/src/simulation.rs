//! Fixed 30 Hz game loop. Events are applied as they arrive; ticks catch up at
//! most three at a time and never change `dt`.

use std::time::{Duration, Instant};

use pqpq_sim::FIXED_DT_SECONDS;
use tokio::sync::watch;

use crate::config::Config;
use crate::rooms::Server;
use crate::transport::EventSource;

const MAX_CATCH_UP: u32 = 3;
/// Beyond this lag the backlog is abandoned and the schedule restarts.
const MAX_LAG: Duration = Duration::from_secs(1);
const STATS_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Default)]
struct Stats {
    ticks: u64,
    late_ticks: u64,
    resets: u64,
    max_tick: Duration,
}

pub async fn run(cfg: Config, mut events: EventSource, mut shutdown: watch::Receiver<bool>) {
    let period = Duration::from_secs_f64(FIXED_DT_SECONDS);
    let mut server = Server::new(cfg);
    let mut next = Instant::now() + period;
    let mut stats = Stats::default();
    let mut stats_at = Instant::now() + STATS_INTERVAL;
    loop {
        tokio::select! {
            _ = shutdown.changed() => {
                server.shutdown();
                return;
            }
            event = events.recv() => match event {
                Some(event) => server.handle(event, Instant::now()),
                None => return,
            },
            _ = tokio::time::sleep_until(next.into()) => {
                let mut ran = 0;
                while Instant::now() >= next && ran < MAX_CATCH_UP {
                    let started = Instant::now();
                    if started - next > period {
                        stats.late_ticks += 1;
                    }
                    server.tick(started);
                    stats.ticks += 1;
                    stats.max_tick = stats.max_tick.max(started.elapsed());
                    next += period;
                    ran += 1;
                }
                if Instant::now().saturating_duration_since(next) > MAX_LAG {
                    next = Instant::now() + period;
                    stats.resets += 1;
                }
                // Only the newest state is sent, never intermediate catch-up ticks.
                server.send_snapshots();
                if Instant::now() >= stats_at {
                    let (conns, players, rooms) = server.counts();
                    eprintln!(
                        "stats: connections={conns} players={players} rooms={rooms} ticks={} late={} resets={} max_tick_us={}",
                        stats.ticks, stats.late_ticks, stats.resets, stats.max_tick.as_micros()
                    );
                    stats = Stats::default();
                    stats_at += STATS_INTERVAL;
                }
            }
        }
    }
}
