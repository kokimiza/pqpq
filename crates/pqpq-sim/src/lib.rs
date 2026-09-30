//! Platform-independent simulation shared by server, terminal and WASM.
//!
//! This crate must not depend on networking, terminal, browser or clock APIs.
//! Time is always passed in explicitly.

mod car;
mod course;
mod sync;
mod world;

pub use car::{CAR_RADIUS, CarState, GateEvent, InputState, MAX_SPEED, Steering, step_car};
pub use course::{Course, Gate, Projection};
pub use sync::{Interpolator, Predictor, TickClock};
pub use world::{LapEvent, WorldCar, step_world};

/// Shared fixed simulation frequency, independent of rendering frequency.
pub const TICK_HZ: u32 = 30;

/// Duration of one simulation step in seconds.
pub const FIXED_DT_SECONDS: f64 = 1.0 / TICK_HZ as f64;

/// Increment when simulation behavior becomes incompatible.
pub const PHYSICS_VERSION: u32 = 1;

pub const COURSE_ID: u32 = 1;
pub const COURSE_VERSION: u32 = 1;

/// Laps needed to finish a race.
pub const LAPS: u32 = 3;

/// Deterministic input sequence used to compare native and WASM builds.
pub fn reference_run() -> CarState {
    let course = Course::standard();
    let mut car = course.grid_slot(0);
    for i in 0..300u32 {
        let steering = match (i / 40) % 3 {
            0 => Steering::Neutral,
            1 => Steering::Left,
            _ => Steering::Right,
        };
        let input = InputState {
            throttle: i % 50 < 40,
            brake: i % 50 >= 45,
            steering,
        };
        step_car(&course, &mut car, input, FIXED_DT_SECONDS);
    }
    car
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_run_is_stable() {
        let car = reference_run();
        // Same vector is checked against the WASM build by web/check.mjs.
        let expected = REFERENCE;
        let got = [
            car.position[0],
            car.position[1],
            car.direction,
            car.velocity[0],
            car.velocity[1],
        ];
        for (g, e) in got.iter().zip(expected) {
            assert!((g - e).abs() < 1e-6, "got {got:?}");
        }
    }

    const REFERENCE: [f64; 5] = [
        12.652196453530346,
        -43.00848023940093,
        1.3063283155510665,
        0.04017250679584599,
        -0.010879175534572292,
    ];
}
