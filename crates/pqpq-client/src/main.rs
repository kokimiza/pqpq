//! pqpq terminal client: `pqpq <username> <room_id>`.
//! Exit codes: 0 normal, 1 connection/protocol/terminal error, 2 bad arguments.

mod app;
mod config;
mod input;
mod network;
mod view;

use std::io::{self, Write};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use crossterm::event::{
    DisableFocusChange, EnableFocusChange, Event, KeyboardEnhancementFlags,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::execute;
use tokio::sync::mpsc;
use tokio::time::MissedTickBehavior;

use crate::app::App;
use crate::config::{ArgsError, Config};
use crate::input::{Command, Keys};
use crate::network::NetEvent;

const FRAME: Duration = Duration::from_micros(16_667);
/// Time allowed for Leave to go out before exiting.
const QUIT_WAIT: Duration = Duration::from_millis(400);

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cfg = match Config::from_args(&args) {
        Ok(cfg) => cfg,
        Err(ArgsError::Usage) => {
            eprintln!("使い方: pqpq <username> <room_id>");
            return ExitCode::from(2);
        },
        Err(ArgsError::Invalid(msg)) => {
            eprintln!("pqpq: {msg}");
            return ExitCode::from(2);
        },
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build();
    match runtime.and_then(|rt| rt.block_on(run(cfg))) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(e) => {
            eprintln!("pqpq: 端末エラー: {e}");
            ExitCode::from(1)
        },
    }
}

/// Undoes terminal modes on normal exit, errors and (via the hook) panics.
struct TerminalModes {
    enhanced: bool,
}

impl TerminalModes {
    fn enter(enhanced: bool) -> io::Result<TerminalModes> {
        let mut out = io::stdout();
        execute!(out, EnableFocusChange)?;
        if enhanced {
            execute!(
                out,
                PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::REPORT_EVENT_TYPES)
            )?;
        }
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            Self::leave(enhanced);
            previous(info);
        }));
        Ok(TerminalModes { enhanced })
    }

    fn leave(enhanced: bool) {
        let mut out = io::stdout();
        if enhanced {
            let _ = execute!(out, PopKeyboardEnhancementFlags);
        }
        let _ = execute!(out, DisableFocusChange);
        let _ = out.flush();
    }
}

impl Drop for TerminalModes {
    fn drop(&mut self) {
        Self::leave(self.enhanced);
        ratatui::restore();
    }
}

async fn run(cfg: Config) -> io::Result<bool> {
    // Windows consoles report key releases natively; elsewhere only terminals
    // with the kitty keyboard protocol do. Others use the timed compat mode.
    let enhanced =
        !cfg!(windows) && crossterm::terminal::supports_keyboard_enhancement().unwrap_or(false);
    let release_events = cfg!(windows) || enhanced;
    let mut terminal = ratatui::try_init()?;
    let _modes = TerminalModes::enter(enhanced)?;

    let (net_tx, net_rx) = mpsc::unbounded_channel();
    let (event_tx, mut events) = mpsc::unbounded_channel();
    tokio::spawn(network::run(cfg.clone(), net_rx, event_tx));
    let (key_tx, mut keys) = mpsc::unbounded_channel();
    std::thread::spawn(move || {
        while let Ok(event) = crossterm::event::read() {
            if key_tx.send(event).is_err() {
                break;
            }
        }
    });

    let mut app = App::new(cfg, Keys::new(release_events), net_tx);
    let mut frames = tokio::time::interval(FRAME);
    frames.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut last = Instant::now();
    let mut focused = true;
    let mut quit_at: Option<Instant> = None;
    loop {
        tokio::select! {
            Some(event) = events.recv() => {
                let closed = matches!(event, NetEvent::Closed(_));
                app.on_net(event, Instant::now());
                if closed && app.quit {
                    break;
                }
            }
            Some(event) = keys.recv() => match event {
                Event::Key(key) => {
                    if let Some(command) = app.keys.on_key(key, Instant::now()) {
                        app.on_command(command);
                        if command == Command::Quit && app.error.is_some() {
                            break;
                        }
                    }
                }
                Event::FocusLost => focused = false,
                Event::FocusGained => focused = true,
                _ => {}
            },
            _ = frames.tick() => {
                let now = Instant::now();
                let size = terminal.size()?;
                app.input_blocked = !focused || size.width < view::MIN_WIDTH || size.height < view::MIN_HEIGHT;
                if app.input_blocked {
                    app.keys.clear();
                }
                app.frame(now, (now - last).as_secs_f64());
                last = now;
                terminal.draw(|f| view::draw(f, &app, now))?;
            }
        }
        if app.quit && *quit_at.get_or_insert_with(|| Instant::now() + QUIT_WAIT) <= Instant::now()
        {
            break;
        }
    }
    Ok(app.error.is_none())
}
