//! Race progression inside one room: countdown, running, finishing, ranking.

use pqpq_protocol::{CarSnapshot, CarStatus, Controls, GridSlot, PlayerId, RaceStart, ResultEntry};
use pqpq_sim::{
    CarState, Course, FIXED_DT_SECONDS, InputState, LAPS, Steering, TICK_HZ, WorldCar, step_car,
    step_world,
};

/// Countdown between reservation and start.
pub const COUNTDOWN_TICKS: u64 = 3 * TICK_HZ as u64;
/// Unfinished racers are DNF after 10 minutes of simulation time.
pub const TIME_LIMIT_TICKS: u64 = 10 * 60 * TICK_HZ as u64;

pub enum Race {
    Waiting { plan: Option<RaceStart> },
    Racing(Run),
    Finished { run: Run, results: Vec<ResultEntry> },
}

pub struct Run {
    pub start: RaceStart,
    /// Sorted by id; kept after disconnects for the result table.
    pub entries: Vec<Entry>,
}

pub struct Entry {
    pub id: PlayerId,
    pub name: String,
    pub status: CarStatus,
    pub car: CarState,
    pub input: InputState,
    /// Race time in seconds.
    pub finish_time: Option<f64>,
    /// Still connected; departed cars vanish from snapshots.
    pub present: bool,
}

impl Run {
    pub fn new(start: RaceStart, names: impl Fn(PlayerId) -> String) -> Run {
        let mut entries: Vec<Entry> = start
            .grid
            .iter()
            .map(|g| Entry {
                id: g.player_id,
                name: names(g.player_id),
                status: CarStatus::Racing,
                car: CarState {
                    position: [g.position[0] as f64, g.position[1] as f64],
                    direction: g.direction as f64,
                    ..Default::default()
                },
                input: InputState::default(),
                finish_time: None,
                present: true,
            })
            .collect();
        entries.sort_by_key(|e| e.id);
        Run { start, entries }
    }

    pub fn entry_mut(&mut self, id: PlayerId) -> Option<&mut Entry> {
        self.entries.iter_mut().find(|e| e.id == id)
    }

    /// One fixed step at `tick`. Returns true when the race is over.
    pub fn step(&mut self, course: &Course, tick: u64) -> bool {
        let mut world: Vec<WorldCar> = self
            .entries
            .iter()
            .filter(|e| e.status == CarStatus::Racing)
            .map(|e| WorldCar {
                id: e.id.0,
                state: e.car,
                input: e.input,
            })
            .collect();
        let laps = step_world(course, &mut world, FIXED_DT_SECONDS);
        for car in &world {
            let entry = self.entry_mut(PlayerId(car.id)).unwrap();
            entry.car = car.state;
        }
        // Finished cars coast out without collisions or lap counting.
        for e in self
            .entries
            .iter_mut()
            .filter(|e| e.status == CarStatus::Finished)
        {
            let mut car = e.car;
            step_car(course, &mut car, InputState::default(), FIXED_DT_SECONDS);
            e.car.position = car.position;
            e.car.velocity = car.velocity;
            e.car.direction = car.direction;
        }
        let elapsed = tick - self.start.start_tick;
        for lap in laps {
            if lap.laps >= LAPS {
                let e = self.entry_mut(PlayerId(lap.id)).unwrap();
                e.status = CarStatus::Finished;
                e.finish_time =
                    Some((elapsed - 1) as f64 * FIXED_DT_SECONDS + lap.fraction * FIXED_DT_SECONDS);
            }
        }
        if elapsed >= TIME_LIMIT_TICKS {
            for e in self
                .entries
                .iter_mut()
                .filter(|e| e.status == CarStatus::Racing)
            {
                e.status = CarStatus::Dnf;
            }
        }
        self.entries.iter().all(|e| e.status != CarStatus::Racing)
    }

    /// Entries in finishing order: finishers by time, then running cars by
    /// progress, then DNF by progress at retirement, ties by PlayerId.
    pub fn standings(&self, course: &Course) -> Vec<&Entry> {
        let class = |e: &Entry| match e.status {
            CarStatus::Finished => 0,
            CarStatus::Racing => 1,
            CarStatus::Dnf => 2,
        };
        let mut keyed: Vec<(&Entry, f64)> = self
            .entries
            .iter()
            .map(|e| (e, e.finish_time.unwrap_or_else(|| -course.progress(&e.car))))
            .collect();
        keyed.sort_by(|(a, ka), (b, kb)| {
            class(a)
                .cmp(&class(b))
                .then(ka.total_cmp(kb))
                .then(a.id.cmp(&b.id))
        });
        keyed.into_iter().map(|(e, _)| e).collect()
    }

    pub fn cars(&self, course: &Course) -> Vec<CarSnapshot> {
        self.standings(course)
            .into_iter()
            .enumerate()
            .filter(|(_, e)| e.present && e.status != CarStatus::Dnf)
            .map(|(rank, e)| CarSnapshot {
                player_id: e.id,
                position: [e.car.position[0] as f32, e.car.position[1] as f32],
                velocity: [e.car.velocity[0] as f32, e.car.velocity[1] as f32],
                direction: e.car.direction as f32,
                laps: e.car.laps.min(u8::MAX as u32) as u8,
                next_checkpoint: e.car.next_checkpoint as u8,
                rank: (rank + 1) as u8,
                status: e.status,
            })
            .collect()
    }

    pub fn results(&self, course: &Course) -> Vec<ResultEntry> {
        self.standings(course)
            .into_iter()
            .map(|e| ResultEntry {
                player_id: e.id,
                username: e.name.clone(),
                status: e.status,
                laps: e.car.laps.min(u8::MAX as u32) as u8,
                finish_time_ms: e.finish_time.map(|t| (t * 1000.0).round() as u32),
            })
            .collect()
    }
}

/// Grid in PlayerId order.
pub fn plan_start(course: &Course, start_id: u64, tick: u64, racers: &[PlayerId]) -> RaceStart {
    let grid = racers
        .iter()
        .enumerate()
        .map(|(i, id)| {
            let slot = course.grid_slot(i);
            GridSlot {
                player_id: *id,
                position: [slot.position[0] as f32, slot.position[1] as f32],
                direction: slot.direction as f32,
            }
        })
        .collect();
    RaceStart {
        start_id,
        start_tick: tick + COUNTDOWN_TICKS,
        server_tick: tick,
        grid,
    }
}

pub fn to_input(c: Controls) -> InputState {
    let steering = match c.steering {
        1 => Steering::Left,
        -1 => Steering::Right,
        _ => Steering::Neutral,
    };
    InputState {
        throttle: c.throttle,
        brake: c.brake,
        steering,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(n: u64) -> (Course, Run) {
        let course = Course::standard();
        let ids: Vec<PlayerId> = (1..=n).map(PlayerId).collect();
        let start = plan_start(&course, 1, 0, &ids);
        let run = Run::new(start, |id| format!("p{}", id.0));
        (course, run)
    }

    #[test]
    fn a_driven_race_finishes_with_ordered_results() {
        let (course, mut r) = run(2);
        // Player 1 drives along the centerline by steering toward it.
        let mut tick = r.start.start_tick;
        let mut done = false;
        while !done && tick < r.start.start_tick + TIME_LIMIT_TICKS + 1 {
            tick += 1;
            let car = r.entries[0].car;
            let p = course.project(car.position);
            let (target, _) = course.point_at(p.s + 15.0);
            let want = (target[1] - car.position[1]).atan2(target[0] - car.position[0]);
            let diff = (want - car.direction + std::f64::consts::PI)
                .rem_euclid(std::f64::consts::TAU)
                - std::f64::consts::PI;
            let steering = if diff > 0.05 {
                1
            } else if diff < -0.05 {
                -1
            } else {
                0
            };
            r.entries[0].input = to_input(Controls {
                throttle: car.speed() < 30.0,
                brake: false,
                steering,
            });
            done = r.step(&course, tick);
        }
        assert!(done);
        let results = r.results(&course);
        assert_eq!(results[0].player_id, PlayerId(1));
        assert_eq!(results[0].status, CarStatus::Finished);
        assert_eq!(results[0].laps, LAPS as u8);
        assert_eq!(results[1].status, CarStatus::Dnf);
        // The limit was hit only by the idle car, long after the finisher.
        let t = results[0].finish_time_ms.unwrap();
        assert!(t > 20_000 && t < 120_000, "{t}");
    }

    #[test]
    fn standings_put_dnf_last_and_break_ties_by_id() {
        let (course, mut r) = run(3);
        r.entries[0].status = CarStatus::Dnf;
        let order: Vec<u64> = r.standings(&course).iter().map(|e| e.id.0).collect();
        assert_eq!(order.last(), Some(&1));
        assert_eq!(r.cars(&course).len(), 2);
    }
}
