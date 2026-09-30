use crate::car::{CarState, GateEvent, add, dot, len, scale, sub};

/// Closed centerline of the only course, in meters, in driving order.
const CENTERLINE: [[f64; 2]; 11] = [
    [-60.0, -50.0],
    [60.0, -50.0],
    [100.0, -30.0],
    [110.0, 10.0],
    [80.0, 40.0],
    [40.0, 30.0],
    [10.0, 55.0],
    [-40.0, 60.0],
    [-90.0, 40.0],
    [-105.0, 0.0],
    [-95.0, -35.0],
];
const HALF_WIDTH: f64 = 7.0;
/// Arc length of the start/finish line from the first centerline point.
const FINISH_S: f64 = 60.0;
/// Intermediate checkpoints, evenly spaced after the finish line.
const CHECKPOINTS: u32 = 4;
const GRID_FIRST_ROW: f64 = 6.0;
const GRID_ROW_GAP: f64 = 7.0;
const GRID_LATERAL: f64 = 3.0;

/// A directed line across the track. Crossing counts only along `tangent`.
#[derive(Clone, Copy, Debug)]
pub struct Gate {
    pub a: [f64; 2],
    pub b: [f64; 2],
    pub tangent: [f64; 2],
    pub s: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct Projection {
    /// Nearest point on the centerline.
    pub point: [f64; 2],
    /// Arc length of `point`.
    pub s: f64,
    pub distance: f64,
}

#[derive(Clone, Debug)]
pub struct Course {
    points: Vec<[f64; 2]>,
    /// Arc length at each point; one extra entry holds the total length.
    cum: Vec<f64>,
    pub length: f64,
    pub half_width: f64,
    gates: Vec<Gate>,
}

/// The only course.
impl Default for Course {
    fn default() -> Self {
        Course::standard()
    }
}

impl Course {
    pub fn standard() -> Course {
        let points = CENTERLINE.to_vec();
        let mut cum = vec![0.0];
        for i in 0..points.len() {
            let next = points[(i + 1) % points.len()];
            cum.push(cum[i] + len(sub(next, points[i])));
        }
        let length = *cum.last().unwrap();
        let mut course = Course {
            points,
            cum,
            length,
            half_width: HALF_WIDTH,
            gates: Vec::new(),
        };
        let n = CHECKPOINTS + 1;
        // Gate order: checkpoints 1..=CHECKPOINTS, then the finish line last.
        course.gates = (1..=n)
            .map(|k| {
                let s = (FINISH_S + length * (k % n) as f64 / n as f64).rem_euclid(length);
                let (p, t) = course.point_at(s);
                let normal = [-t[1], t[0]];
                let reach = HALF_WIDTH + 1.0;
                Gate {
                    a: add(p, scale(normal, reach)),
                    b: sub(p, scale(normal, reach)),
                    tangent: t,
                    s,
                }
            })
            .collect();
        course
    }

    pub fn centerline(&self) -> &[[f64; 2]] {
        &self.points
    }

    /// Checkpoints in order; the last entry is the start/finish line.
    pub fn gates(&self) -> &[Gate] {
        &self.gates
    }

    /// Point and unit tangent at arc length `s`, wrapping around the loop.
    pub fn point_at(&self, s: f64) -> ([f64; 2], [f64; 2]) {
        let s = s.rem_euclid(self.length);
        let n = self.points.len();
        let i = (0..n).rfind(|&i| self.cum[i] <= s).unwrap_or(0);
        let (a, b) = (self.points[i], self.points[(i + 1) % n]);
        let seg = self.cum[i + 1] - self.cum[i];
        let dir = scale(sub(b, a), 1.0 / seg);
        (add(a, scale(dir, s - self.cum[i])), dir)
    }

    pub fn project(&self, p: [f64; 2]) -> Projection {
        let n = self.points.len();
        let mut best = Projection {
            point: self.points[0],
            s: 0.0,
            distance: f64::INFINITY,
        };
        for i in 0..n {
            let (a, b) = (self.points[i], self.points[(i + 1) % n]);
            let ab = sub(b, a);
            let t = (dot(sub(p, a), ab) / dot(ab, ab)).clamp(0.0, 1.0);
            let q = add(a, scale(ab, t));
            let d = len(sub(p, q));
            if d < best.distance {
                best = Projection {
                    point: q,
                    s: self.cum[i] + t * (self.cum[i + 1] - self.cum[i]),
                    distance: d,
                };
            }
        }
        best
    }

    /// Starting position behind the finish line: two columns, rows going back.
    pub fn grid_slot(&self, index: usize) -> CarState {
        let row = (index / 2) as f64;
        let (p, t) = self.point_at(FINISH_S - GRID_FIRST_ROW - row * GRID_ROW_GAP);
        let side = if index.is_multiple_of(2) {
            GRID_LATERAL
        } else {
            -GRID_LATERAL
        };
        CarState {
            position: add(p, scale([-t[1], t[0]], side)),
            direction: t[1].atan2(t[0]),
            ..Default::default()
        }
    }

    /// Checks the movement `before -> car.position` against the next gate only,
    /// so skipped checkpoints, reverse driving and line wiggling never count.
    pub(crate) fn cross_gate(&self, car: &mut CarState, before: [f64; 2]) -> Option<GateEvent> {
        let last = self.gates.len() as u32 - 1;
        let gate = self.gates.get(car.next_checkpoint as usize)?;
        let r = sub(car.position, before);
        if dot(r, gate.tangent) <= 0.0 {
            return None;
        }
        let s = sub(gate.b, gate.a);
        let denom = cross(r, s);
        if denom == 0.0 {
            return None;
        }
        let qp = sub(gate.a, before);
        let t = cross(qp, s) / denom;
        let u = cross(qp, r) / denom;
        if !(0.0..=1.0).contains(&t) || !(0.0..=1.0).contains(&u) {
            return None;
        }
        if car.next_checkpoint == last {
            car.laps += 1;
            car.next_checkpoint = 0;
            Some(GateEvent::Lap { fraction: t })
        } else {
            car.next_checkpoint += 1;
            Some(GateEvent::Checkpoint)
        }
    }

    /// Monotonic race progress: gates passed plus the share of the current
    /// section. Negative section share means behind the previous gate.
    pub fn progress(&self, car: &CarState) -> f64 {
        let n = self.gates.len();
        let next = car.next_checkpoint as usize % n;
        let prev = (next + n - 1) % n;
        let section = (self.gates[next].s - self.gates[prev].s).rem_euclid(self.length);
        let d = (self.project(car.position).s - self.gates[prev].s).rem_euclid(self.length);
        let share = if d <= section {
            d / section
        } else {
            (d - self.length) / section
        };
        (car.laps as usize * n + next) as f64 + share
    }

    /// Left and right track edges for drawing, mitered at each corner.
    pub fn edges(&self) -> (Vec<[f64; 2]>, Vec<[f64; 2]>) {
        let n = self.points.len();
        let normal = |i: usize| {
            let d = sub(self.points[(i + 1) % n], self.points[i]);
            let l = len(d);
            [-d[1] / l, d[0] / l]
        };
        (0..n)
            .map(|i| {
                let (n1, n2) = (normal((i + n - 1) % n), normal(i));
                let bis = add(n1, n2);
                let bis = scale(bis, 1.0 / len(bis));
                let m = self.half_width / dot(bis, n2);
                (
                    add(self.points[i], scale(bis, m)),
                    sub(self.points[i], scale(bis, m)),
                )
            })
            .unzip()
    }

    /// `[min_x, min_y, max_x, max_y]` including the track width.
    pub fn bounds(&self) -> [f64; 4] {
        let mut b = [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        for p in &self.points {
            b = [
                b[0].min(p[0]),
                b[1].min(p[1]),
                b[2].max(p[0]),
                b[3].max(p[1]),
            ];
        }
        let w = self.half_width;
        [b[0] - w, b[1] - w, b[2] + w, b[3] + w]
    }
}

fn cross(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FIXED_DT_SECONDS as DT, InputState, step_car};

    /// Drives the car along the centerline, which is a perfect lap.
    fn drive(course: &Course, car: &mut CarState, from_s: f64, to_s: f64) -> Vec<GateEvent> {
        let mut events = Vec::new();
        let mut s = from_s;
        let step = if to_s > from_s { 1.0 } else { -1.0 };
        while (to_s - s) * step > 0.0 {
            let before = car.position;
            s += step;
            car.position = course.point_at(s).0;
            events.extend(course.cross_gate(car, before));
        }
        events
    }

    #[test]
    fn grid_is_behind_the_line_and_start_crossing_does_not_count() {
        let course = Course::standard();
        let mut car = course.grid_slot(0);
        let s0 = course.project(car.position).s;
        let events = drive(&course, &mut car, s0, FINISH_S + 5.0);
        assert!(events.is_empty());
        assert_eq!(car.laps, 0);
    }

    #[test]
    fn full_lap_counts_once_and_needs_every_checkpoint_in_order() {
        let course = Course::standard();
        let mut car = course.grid_slot(0);
        let s0 = course.project(car.position).s;
        let events = drive(&course, &mut car, s0, FINISH_S + course.length + 1.0);
        assert_eq!(car.laps, 1);
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, GateEvent::Lap { .. }))
                .count(),
            1
        );
        assert_eq!(events.len(), CHECKPOINTS as usize + 1);
    }

    #[test]
    fn wiggling_on_the_line_and_reversing_do_not_count() {
        let course = Course::standard();
        let mut car = course.grid_slot(0);
        let s0 = course.project(car.position).s;
        drive(&course, &mut car, s0, FINISH_S + course.length + 1.0);
        // Back over the line and forward again.
        drive(
            &course,
            &mut car,
            FINISH_S + course.length + 1.0,
            FINISH_S + course.length - 3.0,
        );
        drive(
            &course,
            &mut car,
            FINISH_S + course.length - 3.0,
            FINISH_S + course.length + 3.0,
        );
        assert_eq!(car.laps, 1);
        // Reverse lap: never counts.
        drive(
            &course,
            &mut car,
            FINISH_S + course.length + 3.0,
            FINISH_S + 2.0,
        );
        assert_eq!(car.laps, 1);
    }

    #[test]
    fn fast_car_cannot_tunnel_through_a_gate() {
        let course = Course::standard();
        let g = course.gates()[0];
        let mid = scale(add(g.a, g.b), 0.5);
        let before = sub(mid, scale(g.tangent, 30.0));
        let mut car = CarState {
            position: add(mid, scale(g.tangent, 30.0)),
            ..Default::default()
        };
        assert_eq!(
            course.cross_gate(&mut car, before),
            Some(GateEvent::Checkpoint)
        );
    }

    #[test]
    fn progress_orders_cars() {
        let course = Course::standard();
        let a = course.grid_slot(0);
        let b = course.grid_slot(2);
        assert!(course.progress(&a) > course.progress(&b));
        let mut c = a;
        step_car(
            &course,
            &mut c,
            InputState {
                throttle: true,
                ..Default::default()
            },
            DT,
        );
        assert!(course.progress(&c) > course.progress(&a));
    }
}
