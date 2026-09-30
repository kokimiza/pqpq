//! Terminal key events to held controls.

use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use pqpq_sim::InputState;

/// Without release events a key counts as held this long after its last
/// press or auto-repeat.
const COMPAT_HOLD: Duration = Duration::from_millis(200);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Command {
    Ready,
    Quit,
}

pub struct Keys {
    /// True when the terminal reports key releases.
    pub release_events: bool,
    /// Up, down, left, right: when last pressed, None when released.
    held: [Option<Instant>; 4],
}

impl Keys {
    pub fn new(release_events: bool) -> Self {
        Keys {
            release_events,
            held: [None; 4],
        }
    }

    pub fn on_key(&mut self, key: KeyEvent, now: Instant) -> Option<Command> {
        let ctrl_c =
            key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c');
        let slot = match key.code {
            _ if ctrl_c => None,
            KeyCode::Up | KeyCode::Char('w' | 'W') => Some(0),
            KeyCode::Down | KeyCode::Char('s' | 'S') => Some(1),
            KeyCode::Left | KeyCode::Char('a' | 'A') => Some(2),
            KeyCode::Right | KeyCode::Char('d' | 'D') => Some(3),
            _ => None,
        };
        if let Some(i) = slot {
            self.held[i] = match key.kind {
                KeyEventKind::Release => None,
                KeyEventKind::Press | KeyEventKind::Repeat => Some(now),
            };
            return None;
        }
        if key.kind == KeyEventKind::Release {
            return None;
        }
        match key.code {
            _ if ctrl_c => Some(Command::Quit),
            KeyCode::Char('q' | 'Q') => Some(Command::Quit),
            KeyCode::Enter => Some(Command::Ready),
            _ => None,
        }
    }

    /// Neutral controls, used on focus loss and when the window is too small.
    pub fn clear(&mut self) {
        self.held = [None; 4];
    }

    pub fn state(&self, now: Instant) -> InputState {
        let on = |i: usize| {
            self.held[i].is_some_and(|t| {
                self.release_events || now.saturating_duration_since(t) < COMPAT_HOLD
            })
        };
        InputState::from_keys(on(0), on(1), on(2), on(3))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEventState;

    fn key(code: KeyCode, kind: KeyEventKind) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind,
            state: KeyEventState::NONE,
        }
    }

    #[test]
    fn release_mode_holds_until_release() {
        let mut k = Keys::new(true);
        let t = Instant::now();
        k.on_key(key(KeyCode::Up, KeyEventKind::Press), t);
        assert!(k.state(t + Duration::from_secs(5)).throttle);
        k.on_key(key(KeyCode::Up, KeyEventKind::Release), t);
        assert!(!k.state(t).throttle);
    }

    #[test]
    fn compat_mode_expires() {
        let mut k = Keys::new(false);
        let t = Instant::now();
        k.on_key(key(KeyCode::Char('a'), KeyEventKind::Press), t);
        assert!(k.state(t + Duration::from_millis(100)).steering == pqpq_sim::Steering::Left);
        assert!(k.state(t + Duration::from_millis(300)).steering == pqpq_sim::Steering::Neutral);
    }

    #[test]
    fn commands() {
        let mut k = Keys::new(true);
        let t = Instant::now();
        assert_eq!(
            k.on_key(key(KeyCode::Enter, KeyEventKind::Press), t),
            Some(Command::Ready)
        );
        assert_eq!(
            k.on_key(key(KeyCode::Char('q'), KeyEventKind::Press), t),
            Some(Command::Quit)
        );
        let ctrl_c = KeyEvent {
            modifiers: KeyModifiers::CONTROL,
            ..key(KeyCode::Char('c'), KeyEventKind::Press)
        };
        assert_eq!(k.on_key(ctrl_c, t), Some(Command::Quit));
        assert_eq!(
            k.on_key(key(KeyCode::Enter, KeyEventKind::Release), t),
            None
        );
    }
}
