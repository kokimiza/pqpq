use crate::course::Course;

/// Forward acceleration with throttle, m/s².
const ACCEL: f64 = 14.0;
/// Deceleration while braking, m/s².
const BRAKE: f64 = 30.0;
/// Linear drag, 1/s.
const DRAG: f64 = 0.25;
/// Constant rolling resistance, m/s².
const ROLLING: f64 = 1.0;
pub const MAX_SPEED: f64 = 50.0;
/// Yaw rate at full grip, rad/s.
const TURN_RATE: f64 = 2.0;
/// Speed at which steering reaches full authority, m/s.
const FULL_GRIP_SPEED: f64 = 6.0;
/// Share of sideways velocity (from collisions) kept per step.
const LATERAL_KEEP: f64 = 0.8;
/// Share of along-wall speed kept after touching the boundary.
const WALL_FRICTION: f64 = 0.9;
pub const CAR_RADIUS: f64 = 1.5;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Steering {
    Left,
    #[default]
    Neutral,
    Right,
}

impl Steering {
    /// Left is counter-clockwise (+1) in world coordinates (y up).
    pub fn sign(self) -> f64 {
        match self {
            Steering::Left => 1.0,
            Steering::Neutral => 0.0,
            Steering::Right => -1.0,
        }
    }
}

/// Current controls, rather than key press/release events.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct InputState {
    pub throttle: bool,
    pub brake: bool,
    pub steering: Steering,
}

impl InputState {
    /// Both directions cancel out; brake wins over throttle.
    pub fn from_keys(up: bool, down: bool, left: bool, right: bool) -> Self {
        let steering = match (left, right) {
            (true, false) => Steering::Left,
            (false, true) => Steering::Right,
            _ => Steering::Neutral,
        };
        InputState {
            throttle: up && !down,
            brake: down,
            steering,
        }
    }
}

/// SI units: meters, meters per second and radians.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CarState {
    pub position: [f64; 2],
    pub velocity: [f64; 2],
    pub direction: f64,
    /// Completed laps.
    pub laps: u32,
    /// Index into `Course::gates()`; the last gate is the finish line.
    pub next_checkpoint: u32,
}

impl CarState {
    pub fn speed(&self) -> f64 {
        len(self.velocity)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GateEvent {
    Checkpoint,
    /// `fraction` is where inside the step the finish line was crossed, 0..=1.
    Lap {
        fraction: f64,
    },
}

/// Advances one car by `dt`, including course limits and checkpoint crossing.
/// Car-to-car contact is handled by `step_world`.
pub fn step_car(
    course: &Course,
    car: &mut CarState,
    input: InputState,
    dt: f64,
) -> Option<GateEvent> {
    let heading = [car.direction.cos(), car.direction.sin()];
    let mut speed = dot(car.velocity, heading).max(0.0);
    let along = scale(heading, dot(car.velocity, heading));
    let lateral = sub(car.velocity, along);

    if input.brake {
        speed -= BRAKE * dt;
    } else if input.throttle {
        speed += ACCEL * dt;
    }
    speed -= (DRAG * speed + ROLLING) * dt;
    speed = speed.clamp(0.0, MAX_SPEED);

    let grip = (speed / FULL_GRIP_SPEED).min(1.0);
    car.direction = wrap_angle(car.direction + input.steering.sign() * TURN_RATE * grip * dt);
    let heading = [car.direction.cos(), car.direction.sin()];
    car.velocity = add(scale(heading, speed), scale(lateral, LATERAL_KEEP));

    let before = car.position;
    car.position = add(car.position, scale(car.velocity, dt));
    constrain(course, car);
    course.cross_gate(car, before)
}

/// Pushes the car back inside the track and removes outward velocity.
pub(crate) fn constrain(course: &Course, car: &mut CarState) {
    let p = course.project(car.position);
    if p.distance <= course.half_width {
        return;
    }
    let normal = scale(sub(car.position, p.point), 1.0 / p.distance);
    car.position = add(p.point, scale(normal, course.half_width));
    let outward = dot(car.velocity, normal);
    if outward > 0.0 {
        car.velocity = scale(sub(car.velocity, scale(normal, outward)), WALL_FRICTION);
    }
}

pub(crate) fn wrap_angle(a: f64) -> f64 {
    let tau = std::f64::consts::TAU;
    let a = a.rem_euclid(tau);
    if a > std::f64::consts::PI { a - tau } else { a }
}

pub(crate) fn add(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] + b[0], a[1] + b[1]]
}
pub(crate) fn sub(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] - b[0], a[1] - b[1]]
}
pub(crate) fn scale(a: [f64; 2], k: f64) -> [f64; 2] {
    [a[0] * k, a[1] * k]
}
pub(crate) fn dot(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}
pub(crate) fn len(a: [f64; 2]) -> f64 {
    dot(a, a).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FIXED_DT_SECONDS as DT;

    #[test]
    fn brake_wins_and_steering_cancels() {
        let i = InputState::from_keys(true, true, true, true);
        assert_eq!(
            i,
            InputState {
                throttle: false,
                brake: true,
                steering: Steering::Neutral
            }
        );
    }

    #[test]
    fn speed_is_capped_and_never_negative() {
        let course = Course::standard();
        let mut car = course.grid_slot(0);
        let go = InputState {
            throttle: true,
            ..Default::default()
        };
        for _ in 0..60 {
            step_car(&course, &mut car, go, DT);
            assert!(car.speed() <= MAX_SPEED + 1e-9);
        }
        let stop = InputState {
            brake: true,
            ..Default::default()
        };
        for _ in 0..200 {
            step_car(&course, &mut car, stop, DT);
        }
        assert_eq!(car.speed(), 0.0);
    }

    #[test]
    fn car_stays_on_track() {
        let course = Course::standard();
        let mut car = course.grid_slot(0);
        let input = InputState {
            throttle: true,
            steering: Steering::Left,
            ..Default::default()
        };
        for _ in 0..600 {
            step_car(&course, &mut car, input, DT);
            assert!(course.project(car.position).distance <= course.half_width + 1e-9);
        }
    }
}
