//! Client-side prediction, reconciliation and interpolation shared by the
//! terminal and the browser. Callers pass local time in seconds.

use std::collections::VecDeque;

use crate::car::{CarState, InputState, add, len, scale, step_car, sub, wrap_angle};
use crate::course::Course;
use crate::{FIXED_DT_SECONDS, TICK_HZ};

/// Unacknowledged inputs kept for replay; beyond this prediction pauses.
pub const HISTORY_LIMIT: usize = 120;
/// Errors larger than this are snapped instead of smoothed, in meters.
const SNAP_DISTANCE: f64 = 4.0;
/// Time to hide a small correction.
const SMOOTHING_SECONDS: f64 = 0.1;
const INTERPOLATION_LIMIT: usize = 30;
/// 100 ms of velocity extrapolation at most.
pub const MAX_EXTRAPOLATION_TICKS: f64 = 3.0;

#[derive(Clone, Debug, Default)]
pub struct Predictor {
    active: bool,
    state: CarState,
    previous: CarState,
    history: VecDeque<(u64, InputState)>,
    correction: [f64; 2],
    correction_dir: f64,
}

impl Predictor {
    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn stop(&mut self) {
        *self = Self::default();
    }

    /// One fixed step with the input sent under `sequence`.
    pub fn step(&mut self, course: &Course, sequence: u64, input: InputState) {
        if !self.active {
            return;
        }
        if self.history.len() >= HISTORY_LIMIT {
            // The server stopped acknowledging; restart from the next snapshot.
            self.stop();
            return;
        }
        self.history.push_back((sequence, input));
        self.previous = self.state;
        step_car(course, &mut self.state, input, FIXED_DT_SECONDS);
    }

    /// Rebases on an authoritative state and replays unacknowledged inputs.
    /// Returns the prediction error in meters.
    pub fn reconcile(&mut self, course: &Course, server: CarState, last_processed: u64) -> f64 {
        while self
            .history
            .front()
            .is_some_and(|(seq, _)| *seq <= last_processed)
        {
            self.history.pop_front();
        }
        let mut replayed = server;
        let mut previous = server;
        for (_, input) in &self.history {
            previous = replayed;
            step_car(course, &mut replayed, *input, FIXED_DT_SECONDS);
        }
        if !self.active {
            *self = Predictor {
                active: true,
                state: replayed,
                previous,
                ..Default::default()
            };
            return 0.0;
        }
        let err = sub(self.state.position, replayed.position);
        let err_dir = wrap_angle(self.state.direction - replayed.direction);
        let error = len(err);
        if error > SNAP_DISTANCE {
            self.correction = [0.0; 2];
            self.correction_dir = 0.0;
        } else {
            self.correction = add(self.correction, err);
            self.correction_dir += err_dir;
        }
        self.state = replayed;
        self.previous = previous;
        error
    }

    /// Fades the visual correction; call once per rendered frame.
    pub fn decay(&mut self, dt_seconds: f64) {
        let keep = (-dt_seconds * 3.0 / SMOOTHING_SECONDS).exp();
        self.correction = scale(self.correction, keep);
        self.correction_dir *= keep;
    }

    /// Display state, `alpha` in 0..=1 between the last two fixed steps.
    pub fn render(&self, alpha: f64) -> CarState {
        let mut car = lerp(&self.previous, &self.state, alpha);
        car.position = add(car.position, self.correction);
        car.direction = wrap_angle(car.direction + self.correction_dir);
        car
    }

    /// Latest predicted (not smoothed) state.
    pub fn state(&self) -> CarState {
        self.state
    }
}

/// Snapshot buffer for drawing other cars slightly in the past.
#[derive(Clone, Debug, Default)]
pub struct Interpolator {
    frames: VecDeque<(u64, Vec<(u64, CarState)>)>,
}

impl Interpolator {
    pub fn push(&mut self, tick: u64, cars: Vec<(u64, CarState)>) {
        if self.frames.back().is_some_and(|(t, _)| *t >= tick) {
            return;
        }
        self.frames.push_back((tick, cars));
        while self.frames.len() > INTERPOLATION_LIMIT {
            self.frames.pop_front();
        }
    }

    pub fn clear(&mut self) {
        self.frames.clear();
    }

    pub fn latest_tick(&self) -> Option<u64> {
        self.frames.back().map(|(t, _)| *t)
    }

    /// True once `render_tick` is further ahead than extrapolation covers.
    pub fn is_stale(&self, render_tick: f64) -> bool {
        self.latest_tick()
            .is_some_and(|t| render_tick - t as f64 > MAX_EXTRAPOLATION_TICKS)
    }

    pub fn sample(&self, render_tick: f64) -> Vec<(u64, CarState)> {
        let Some((last_tick, last)) = self.frames.back() else {
            return Vec::new();
        };
        if render_tick >= *last_tick as f64 {
            let ahead =
                (render_tick - *last_tick as f64).min(MAX_EXTRAPOLATION_TICKS) / TICK_HZ as f64;
            return last
                .iter()
                .map(|(id, c)| {
                    (
                        *id,
                        CarState {
                            position: add(c.position, scale(c.velocity, ahead)),
                            ..*c
                        },
                    )
                })
                .collect();
        }
        let next = self
            .frames
            .iter()
            .position(|(t, _)| *t as f64 > render_tick)
            .unwrap_or(0);
        if next == 0 {
            return self.frames[0].1.clone();
        }
        let (ta, a) = &self.frames[next - 1];
        let (tb, b) = &self.frames[next];
        let alpha = (render_tick - *ta as f64) / (*tb - *ta) as f64;
        b.iter()
            .map(|(id, cb)| match a.iter().find(|(ida, _)| ida == id) {
                Some((_, ca)) => (*id, lerp(ca, cb, alpha)),
                None => (*id, *cb),
            })
            .collect()
    }
}

/// Estimates the server tick from snapshot arrival times without relying on
/// synchronized wall clocks.
#[derive(Clone, Copy, Debug, Default)]
pub struct TickClock {
    /// Server time minus local time, seconds.
    offset: Option<f64>,
}

impl TickClock {
    pub fn observe(&mut self, tick: u64, now_s: f64) {
        let sample = tick as f64 / TICK_HZ as f64 - now_s;
        self.offset = Some(match self.offset {
            // Late packets only pull the estimate back slowly; a jump of more
            // than a second (tab sleep, server restart) resets it.
            Some(o) if (sample - o).abs() < 1.0 => {
                o + (sample - o) * if sample > o { 0.5 } else { 0.02 }
            },
            _ => sample,
        });
    }

    /// Latest tick expected to have arrived by `now_s`.
    pub fn tick_at(&self, now_s: f64) -> Option<f64> {
        self.offset.map(|o| (now_s + o) * TICK_HZ as f64)
    }
}

fn lerp(a: &CarState, b: &CarState, t: f64) -> CarState {
    let mix = |x: [f64; 2], y: [f64; 2]| add(x, scale(sub(y, x), t));
    CarState {
        position: mix(a.position, b.position),
        velocity: mix(a.velocity, b.velocity),
        direction: wrap_angle(a.direction + wrap_angle(b.direction - a.direction) * t),
        ..*b
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Steering;

    #[test]
    fn reconcile_with_matching_server_state_has_no_error() {
        let course = Course::standard();
        let start = course.grid_slot(0);
        let go = InputState {
            throttle: true,
            steering: Steering::Left,
            ..Default::default()
        };
        let mut p = Predictor::default();
        p.reconcile(&course, start, 0);
        for seq in 1..=10 {
            p.step(&course, seq, go);
        }
        // Server processed 1..=4 exactly like the client did.
        let mut server = start;
        for _ in 0..4 {
            step_car(&course, &mut server, go, FIXED_DT_SECONDS);
        }
        let err = p.reconcile(&course, server, 4);
        assert!(err < 1e-9);
        assert_eq!(p.history.len(), 6);
    }

    #[test]
    fn large_error_snaps_and_small_error_is_smoothed() {
        let course = Course::standard();
        let start = course.grid_slot(0);
        let mut p = Predictor::default();
        p.reconcile(&course, start, 0);
        let mut off = start;
        off.position[0] += 1.0;
        assert!((p.reconcile(&course, off, 0) - 1.0).abs() < 1e-9);
        assert!((p.render(1.0).position[0] - start.position[0]).abs() < 1e-9);
        p.decay(1.0);
        assert!((p.render(1.0).position[0] - off.position[0]).abs() < 1e-6);
        off.position[0] += 10.0;
        p.reconcile(&course, off, 0);
        assert_eq!(p.render(1.0).position, off.position);
    }

    #[test]
    fn prediction_pauses_when_history_overflows() {
        let course = Course::standard();
        let mut p = Predictor::default();
        p.reconcile(&course, course.grid_slot(0), 0);
        for seq in 1..=HISTORY_LIMIT as u64 + 1 {
            p.step(&course, seq, InputState::default());
        }
        assert!(!p.is_active());
    }

    #[test]
    fn interpolates_between_and_extrapolates_briefly() {
        let car = |x: f64| CarState {
            position: [x, 0.0],
            velocity: [30.0, 0.0],
            ..Default::default()
        };
        let mut i = Interpolator::default();
        i.push(10, vec![(1, car(0.0))]);
        i.push(12, vec![(1, car(2.0))]);
        assert!((i.sample(11.0)[0].1.position[0] - 1.0).abs() < 1e-9);
        // 30 m/s for at most 3 ticks = 3 m.
        assert!((i.sample(20.0)[0].1.position[0] - 5.0).abs() < 1e-9);
        assert!(i.is_stale(20.0));
    }

    #[test]
    fn angle_interpolation_takes_the_short_way() {
        let a = CarState {
            direction: 3.0,
            ..Default::default()
        };
        let b = CarState {
            direction: -3.0,
            ..Default::default()
        };
        assert!(lerp(&a, &b, 0.5).direction.abs() > 3.0);
    }

    #[test]
    fn clock_prefers_fast_packets() {
        let mut c = TickClock::default();
        c.observe(300, 0.0);
        c.observe(300, 0.1); // late packet barely moves the estimate
        assert!((c.tick_at(0.0).unwrap() - 300.0).abs() < 0.2);
    }
}
