//! Browser/WASM boundary. JS owns DOM, Canvas, keyboard and WebTransport I/O;
//! everything game-related runs here. u64 ids cross as strings, never Numbers.

mod app;

use js_sys::{Array, Object, Reflect};
use pqpq_protocol::{
    CarStatus, ClientMessage, Join, PROTOCOL_VERSION, validate_room_id, validate_username,
};
use pqpq_sim::{COURSE_ID, COURSE_VERSION, Course, PHYSICS_VERSION};
use wasm_bindgen::prelude::*;

pub use app::{Screen, Session};

#[wasm_bindgen]
pub fn protocol_version() -> u32 {
    pqpq_protocol::PROTOCOL_VERSION
}

#[wasm_bindgen]
pub fn physics_version() -> u32 {
    pqpq_sim::PHYSICS_VERSION
}

#[wasm_bindgen]
pub fn tick_hz() -> u32 {
    pqpq_sim::TICK_HZ
}

#[wasm_bindgen]
pub fn fixed_dt_seconds() -> f64 {
    pqpq_sim::FIXED_DT_SECONDS
}

/// Form check with the same rules as the server; `undefined` when valid.
#[wasm_bindgen]
pub fn validate(username: &str, room_id: &str) -> Option<String> {
    validate_username(username)
        .and(validate_room_id(room_id))
        .err()
        .map(|e| e.to_string())
}

/// Same input sequence as the native test, for web/check.mjs.
#[wasm_bindgen]
pub fn reference_state() -> Vec<f64> {
    let c = pqpq_sim::reference_run();
    vec![
        c.position[0],
        c.position[1],
        c.direction,
        c.velocity[0],
        c.velocity[1],
    ]
}

/// Known Join bytes, for web/check.mjs.
#[wasm_bindgen]
pub fn reference_join_frame() -> Vec<u8> {
    ClientMessage::Join(Join {
        protocol_version: PROTOCOL_VERSION,
        username: "foo".into(),
        room_id: "1234".into(),
        course_id: COURSE_ID,
        course_version: COURSE_VERSION,
        physics_version: PHYSICS_VERSION,
    })
    .encode_frame()
}

/// Track geometry for drawing, straight from the shared simulation.
#[wasm_bindgen]
pub fn course_geometry() -> JsValue {
    let course = Course::standard();
    let (left, right) = course.edges();
    let flat = |pts: &[[f64; 2]]| pts.iter().flatten().copied().collect::<Vec<f64>>();
    let finish = course.gates().last().unwrap();
    object(&[
        ("left", js_sys::Float64Array::from(&flat(&left)[..]).into()),
        (
            "right",
            js_sys::Float64Array::from(&flat(&right)[..]).into(),
        ),
        (
            "finish",
            js_sys::Float64Array::from(&[finish.a[0], finish.a[1], finish.b[0], finish.b[1]][..])
                .into(),
        ),
        (
            "bounds",
            js_sys::Float64Array::from(&course.bounds()[..]).into(),
        ),
        ("carRadius", pqpq_sim::CAR_RADIUS.into()),
    ])
}

fn object(fields: &[(&str, JsValue)]) -> JsValue {
    let o = Object::new();
    for (k, v) in fields {
        let _ = Reflect::set(&o, &JsValue::from_str(k), v);
    }
    o.into()
}

fn opt<T: Into<JsValue>>(v: Option<T>) -> JsValue {
    v.map_or(JsValue::UNDEFINED, Into::into)
}

#[wasm_bindgen]
pub struct Client {
    session: Session,
    left: bool,
}

#[wasm_bindgen]
impl Client {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Client {
        Client {
            session: Session::new(),
            left: false,
        }
    }

    pub fn join(&mut self, username: &str, room_id: &str) -> Result<(), JsError> {
        self.session
            .join(username, room_id)
            .map_err(|e| JsError::new(&e))
    }

    pub fn ready(&mut self) {
        self.session.ready();
    }

    pub fn leave(&mut self) {
        self.left = true;
        self.session.leave();
    }

    pub fn on_stream(&mut self, bytes: &[u8], now_ms: f64) -> Result<(), JsError> {
        self.session
            .on_stream(bytes, now_ms)
            .map_err(|e| JsError::new(&e))
    }

    pub fn on_datagram(&mut self, bytes: &[u8], now_ms: f64) {
        self.session.on_datagram(bytes, now_ms);
    }

    /// Transport ended; shown unless the player left on purpose.
    pub fn closed(&mut self, reason: &str) {
        if !self.left {
            self.session.fail(format!(
                "{reason}。再読み込みすると新しいプレイヤーとして参加できます"
            ));
        }
    }

    pub fn set_keys(&mut self, up: bool, down: bool, left: bool, right: bool) {
        self.session.set_keys(up, down, left, right);
    }

    pub fn neutral(&mut self) {
        self.session.neutral();
    }

    pub fn update(&mut self, now_ms: f64) {
        self.session.update(now_ms);
    }

    /// Bytes for the control stream (possibly empty).
    pub fn take_stream(&mut self) -> Vec<u8> {
        self.session.take_stream()
    }

    pub fn take_datagram(&mut self) -> Option<Vec<u8>> {
        self.session.take_datagram()
    }

    /// Flat `[labelIndex, x, y, direction, isMe]` per car, labelIndex -1 if none.
    pub fn cars(&self, now_ms: f64) -> Vec<f64> {
        let Some(view) = &self.session.view else {
            return Vec::new();
        };
        self.session
            .cars(now_ms)
            .iter()
            .flat_map(|c| {
                let index = view.label_index(c.id).map_or(-1.0, |i| i as f64);
                [
                    index,
                    c.state.position[0],
                    c.state.position[1],
                    c.state.direction,
                    c.me as u8 as f64,
                ]
            })
            .collect()
    }

    /// Everything the page shows, as a plain object.
    pub fn view(&self, now_ms: f64) -> JsValue {
        let s = &self.session;
        let screen = s.screen();
        let mut fields: Vec<(&str, JsValue)> = vec![
            ("screen", screen.name().into()),
            ("error", opt(s.error.clone())),
            ("notice", opt(s.notice().map(str::to_owned))),
            ("rtt", opt(s.rtt_ms.map(f64::round))),
        ];
        let Some(view) = &s.view else {
            return object(&fields);
        };
        let label = |id| view.label_of(id).to_string();
        let roster = Array::new();
        for e in &view.roster {
            roster.push(&object(&[
                ("id", e.player_id.0.to_string().into()),
                ("name", e.username.clone().into()),
                ("ready", e.ready.into()),
                ("racer", (e.role == pqpq_protocol::Role::Racer).into()),
                ("me", (e.player_id == view.me).into()),
            ]));
        }
        let standings = Array::new();
        if let Some(snap) = &s.last_snapshot {
            let mut cars = snap.cars.clone();
            cars.sort_by_key(|c| c.rank);
            for c in cars {
                standings.push(&object(&[
                    ("rank", c.rank.into()),
                    ("label", label(c.player_id).into()),
                    ("name", view.name_of(c.player_id).unwrap_or("?").into()),
                    ("status", status(c.status).into()),
                    ("lap", (c.laps + 1).min(view.laps).into()),
                    ("me", (c.player_id == view.me).into()),
                ]));
            }
        }
        let results = Array::new();
        for (i, r) in view.results.iter().enumerate() {
            results.push(&object(&[
                ("rank", ((i + 1) as u32).into()),
                ("label", label(r.player_id).into()),
                ("name", r.username.clone().into()),
                ("status", status(r.status).into()),
                ("timeMs", opt(r.finish_time_ms)),
                ("laps", r.laps.into()),
                ("me", (r.player_id == view.me).into()),
            ]));
        }
        let mine = s.my_snapshot();
        fields.extend([
            ("room", view.room_id.clone().into()),
            ("me", view.me.0.to_string().into()),
            ("meLabel", label(view.me).into()),
            ("minRacers", view.min_racers.into()),
            ("laps", view.laps.into()),
            ("roster", roster.into()),
            ("standings", standings.into()),
            ("results", results.into()),
            ("countdown", opt(s.countdown(now_ms))),
            ("time", opt(s.race_time(now_ms))),
            ("stale", s.stale(now_ms).into()),
            (
                "entrants",
                opt(s.last_snapshot.as_ref().map(|x| x.entrants)),
            ),
            ("rank", opt(mine.map(|c| c.rank))),
            ("lap", opt(mine.map(|c| (c.laps + 1).min(view.laps)))),
            (
                "finished",
                mine.is_some_and(|c| c.status == CarStatus::Finished).into(),
            ),
            (
                "speedKmh",
                opt(mine.map(|c| (c.velocity[0].hypot(c.velocity[1]) * 3.6).round())),
            ),
        ]);
        object(&fields)
    }
}

impl Default for Client {
    fn default() -> Self {
        Self::new()
    }
}

fn status(s: CarStatus) -> &'static str {
    match s {
        CarStatus::Racing => "RUN",
        CarStatus::Finished => "FIN",
        CarStatus::Dnf => "DNF",
    }
}
