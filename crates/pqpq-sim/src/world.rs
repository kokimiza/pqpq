use crate::car::{
    CAR_RADIUS, CarState, GateEvent, InputState, add, constrain, dot, len, scale, step_car, sub,
};
use crate::course::Course;

/// Bounciness of car-to-car contact.
const RESTITUTION: f64 = 0.4;

#[derive(Clone, Copy, Debug)]
pub struct WorldCar {
    pub id: u64,
    pub state: CarState,
    pub input: InputState,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LapEvent {
    pub id: u64,
    pub laps: u32,
    /// Where inside the step the finish line was crossed, 0..=1.
    pub fraction: f64,
}

/// Advances every running car by one step. `cars` must be sorted by id so the
/// result never depends on map iteration order. Finished cars are not passed.
pub fn step_world(course: &Course, cars: &mut [WorldCar], dt: f64) -> Vec<LapEvent> {
    let mut laps = Vec::new();
    for car in cars.iter_mut() {
        if let Some(GateEvent::Lap { fraction }) = step_car(course, &mut car.state, car.input, dt) {
            laps.push(LapEvent {
                id: car.id,
                laps: car.state.laps,
                fraction,
            });
        }
    }
    // ponytail: O(n²) pairs, fine for the 32-car room cap; add a grid if rooms grow.
    for i in 0..cars.len() {
        for j in i + 1..cars.len() {
            let (left, right) = cars.split_at_mut(j);
            collide(&mut left[i].state, &mut right[0].state);
        }
    }
    for car in cars.iter_mut() {
        constrain(course, &mut car.state);
    }
    laps
}

fn collide(a: &mut CarState, b: &mut CarState) {
    let d = sub(b.position, a.position);
    let dist = len(d);
    let overlap = 2.0 * CAR_RADIUS - dist;
    if overlap <= 0.0 {
        return;
    }
    // Exactly stacked cars are pushed apart along x.
    let n = if dist > 1e-9 {
        scale(d, 1.0 / dist)
    } else {
        [1.0, 0.0]
    };
    a.position = sub(a.position, scale(n, overlap / 2.0));
    b.position = add(b.position, scale(n, overlap / 2.0));
    let closing = dot(sub(b.velocity, a.velocity), n);
    if closing < 0.0 {
        let impulse = -(1.0 + RESTITUTION) * closing / 2.0;
        a.velocity = sub(a.velocity, scale(n, impulse));
        b.velocity = add(b.velocity, scale(n, impulse));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FIXED_DT_SECONDS as DT;

    #[test]
    fn overlapping_cars_are_separated() {
        let course = Course::standard();
        let mut s = course.grid_slot(0);
        s.velocity = [5.0, 0.0];
        let mut other = s;
        other.position[0] += 1.0;
        other.velocity = [0.0, 0.0];
        let mut cars = [
            WorldCar {
                id: 1,
                state: s,
                input: InputState::default(),
            },
            WorldCar {
                id: 2,
                state: other,
                input: InputState::default(),
            },
        ];
        step_world(&course, &mut cars, DT);
        assert!(
            len(sub(cars[0].state.position, cars[1].state.position)) >= 2.0 * CAR_RADIUS - 1e-6
        );
    }

    #[test]
    fn same_inputs_give_same_world() {
        let course = Course::standard();
        let make = || {
            (0..6)
                .map(|i| WorldCar {
                    id: i,
                    state: course.grid_slot(i as usize),
                    input: InputState {
                        throttle: true,
                        ..Default::default()
                    },
                })
                .collect::<Vec<_>>()
        };
        let (mut a, mut b) = (make(), make());
        for _ in 0..200 {
            step_world(&course, &mut a, DT);
            step_world(&course, &mut b, DT);
        }
        for (x, y) in a.iter().zip(&b) {
            assert_eq!(x.state, y.state);
        }
    }
}
