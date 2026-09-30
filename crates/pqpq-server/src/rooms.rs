//! All rooms, players and connections. Owned by the game loop alone, so room
//! creation, joins and leaves are never concurrent.

use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant};

use pqpq_protocol::{
    CancelReason, CarStatus, ClientMessage, ErrorCode, GameSnapshot, InputDatagram, Join, Joined,
    LeaveReason, PROTOCOL_VERSION, PlayerId, RacePhase, Role, RosterEntry, ServerMessage,
    fragment_snapshot, validate_room_id, validate_username,
};
use pqpq_sim::{COURSE_ID, COURSE_VERSION, Course, InputState, LAPS, PHYSICS_VERSION, TICK_HZ};

use crate::config::Config;
use crate::race::{Race, Run, plan_start, to_input};
use crate::transport::{ConnectionHandle, Event};

const JOIN_TIMEOUT: Duration = Duration::from_secs(5);
const PING_INTERVAL: Duration = Duration::from_secs(1);
const PONG_TIMEOUT: Duration = Duration::from_secs(5);
/// Controls go neutral when no input arrived for this long.
const INPUT_TIMEOUT: Duration = Duration::from_millis(250);

struct Conn {
    handle: ConnectionHandle,
    joined: Option<(String, PlayerId)>,
    since: Instant,
    last_ping: Instant,
    /// Outstanding ping nonce and send time.
    ping: Option<(u64, Instant)>,
}

struct Player {
    name: String,
    role: Role,
    ready: bool,
    conn: ConnectionHandle,
    /// Newest input sequence consumed (applied or discarded).
    last_seq: u64,
    input: InputState,
    last_input: Option<Instant>,
}

struct Room {
    players: BTreeMap<PlayerId, Player>,
    revision: u64,
    race: Race,
}

impl Room {
    fn broadcast(&self, msg: &ServerMessage) {
        let frame = msg.encode_frame();
        self.players
            .values()
            .for_each(|p| p.conn.send_reliable(&frame));
    }

    fn roster(&self) -> Vec<RosterEntry> {
        self.players
            .iter()
            .map(|(id, p)| RosterEntry {
                player_id: *id,
                username: p.name.clone(),
                role: p.role,
                ready: p.ready,
            })
            .collect()
    }
}

pub struct Server {
    cfg: Config,
    course: Course,
    rooms: BTreeMap<String, Room>,
    conns: HashMap<u64, Conn>,
    players: usize,
    next_player_id: u64,
    next_start_id: u64,
    next_nonce: u64,
    pub tick: u64,
}

impl Server {
    pub fn new(cfg: Config) -> Server {
        Server {
            cfg,
            course: Course::standard(),
            rooms: BTreeMap::new(),
            conns: HashMap::new(),
            players: 0,
            next_player_id: 1,
            next_start_id: 1,
            next_nonce: 1,
            tick: 0,
        }
    }

    pub fn counts(&self) -> (usize, usize, usize) {
        (self.conns.len(), self.players, self.rooms.len())
    }

    pub fn handle(&mut self, event: Event, now: Instant) {
        match event {
            Event::Connected(handle) => {
                if !handle.is_closed() {
                    let conn = Conn {
                        handle,
                        joined: None,
                        since: now,
                        last_ping: now,
                        ping: None,
                    };
                    self.conns.insert(conn.handle.id(), conn);
                }
            },
            Event::Reliable(id, body) => {
                if !self.conns.contains_key(&id) {
                    return;
                }
                match ClientMessage::decode(&body) {
                    Ok(msg) => self.on_message(id, msg),
                    Err(_) => self.reject(id, ErrorCode::ProtocolViolation),
                }
            },
            Event::Disconnected(id) => {
                if let Some(Conn {
                    joined: Some((room, pid)),
                    ..
                }) = self.conns.remove(&id)
                {
                    self.remove_player(&room, pid, LeaveReason::Disconnected);
                }
            },
        }
    }

    fn on_message(&mut self, id: u64, msg: ClientMessage) {
        let conn = &self.conns[&id];
        let joined = conn.joined.clone();
        match (msg, joined) {
            (ClientMessage::Join(join), None) => self.join(id, join),
            (ClientMessage::Ping { nonce }, _) => conn
                .handle
                .send_reliable(&ServerMessage::Pong { nonce }.encode_frame()),
            (ClientMessage::Pong { nonce }, _) => {
                let conn = self.conns.get_mut(&id).unwrap();
                if conn.ping.is_some_and(|(n, _)| n == nonce) {
                    conn.ping = None;
                }
            },
            (ClientMessage::Ready, Some((room, pid))) => self.ready(id, &room, pid),
            (ClientMessage::Leave, joined) => {
                let conn = self.conns.remove(&id).unwrap();
                conn.handle.close();
                if let Some((room, pid)) = joined {
                    self.remove_player(&room, pid, LeaveReason::Left);
                }
            },
            // Second Join, or anything but Join before joining.
            _ => self.reject(id, ErrorCode::ProtocolViolation),
        }
    }

    /// Sends a fatal error, closes the connection and forgets it now.
    fn reject(&mut self, id: u64, code: ErrorCode) {
        let Some(conn) = self.conns.remove(&id) else {
            return;
        };
        conn.handle
            .send_reliable(&ServerMessage::error(code, true).encode_frame());
        conn.handle.close();
        if let Some((room, pid)) = conn.joined {
            self.remove_player(&room, pid, LeaveReason::Disconnected);
        }
    }

    fn join(&mut self, id: u64, join: Join) {
        if join.protocol_version != PROTOCOL_VERSION
            || join.course_id != COURSE_ID
            || join.course_version != COURSE_VERSION
            || join.physics_version != PHYSICS_VERSION
        {
            return self.reject(id, ErrorCode::VersionMismatch);
        }
        if validate_username(&join.username).is_err() {
            return self.reject(id, ErrorCode::InvalidUsername);
        }
        if validate_room_id(&join.room_id).is_err() {
            return self.reject(id, ErrorCode::InvalidRoomId);
        }
        if self.players >= self.cfg.max_connections {
            return self.reject(id, ErrorCode::ServerFull);
        }
        match self.rooms.get(&join.room_id) {
            Some(room) if room.players.len() >= self.cfg.max_room_players => {
                return self.reject(id, ErrorCode::RoomFull);
            },
            None if self.rooms.len() >= self.cfg.max_rooms => {
                return self.reject(id, ErrorCode::ServerFull);
            },
            _ => {},
        }
        // PlayerIds are never reused; exhaustion refuses instead of wrapping.
        let Some(next) = self.next_player_id.checked_add(1) else {
            return self.reject(id, ErrorCode::ServerFull);
        };
        let pid = PlayerId(self.next_player_id);
        self.next_player_id = next;

        let room_id = join.room_id;
        let room = self.rooms.entry(room_id.clone()).or_insert_with(|| Room {
            players: BTreeMap::new(),
            revision: 0,
            race: Race::Waiting { plan: None },
        });
        cancel_countdown(room);
        let role = if matches!(room.race, Race::Waiting { .. }) {
            Role::Racer
        } else {
            Role::Spectator
        };
        let handle = self.conns[&id].handle.clone();
        let player = Player {
            name: join.username,
            role,
            ready: false,
            conn: handle.clone(),
            last_seq: 0,
            input: InputState::default(),
            last_input: None,
        };
        room.revision += 1;
        room.broadcast(&ServerMessage::PlayerJoined {
            entry: RosterEntry {
                player_id: pid,
                username: player.name.clone(),
                role,
                ready: false,
            },
            room_revision: room.revision,
        });
        room.players.insert(pid, player);

        let (phase, start, results) = match &room.race {
            Race::Waiting { plan } => (RacePhase::Waiting, plan.clone(), Vec::new()),
            Race::Racing(run) => (RacePhase::Racing, Some(run.start.clone()), Vec::new()),
            Race::Finished { run, results, .. } => (
                RacePhase::Finished,
                Some(run.start.clone()),
                results.clone(),
            ),
        };
        let joined = Joined {
            player_id: pid,
            room_id: room_id.clone(),
            role,
            roster: room.roster(),
            room_revision: room.revision,
            phase,
            server_tick: self.tick,
            start,
            course_id: COURSE_ID,
            course_version: COURSE_VERSION,
            physics_version: PHYSICS_VERSION,
            tick_hz: TICK_HZ as u16,
            laps: LAPS as u8,
            min_racers: self.cfg.min_racers.min(u8::MAX as usize) as u8,
            results,
        };
        handle.send_reliable(&ServerMessage::Joined(joined).encode_frame());
        self.conns.get_mut(&id).unwrap().joined = Some((room_id, pid));
        self.players += 1;
    }

    fn ready(&mut self, id: u64, room_id: &str, pid: PlayerId) {
        let room = self.rooms.get_mut(room_id).unwrap();
        let player = room.players.get_mut(&pid).unwrap();
        if !matches!(room.race, Race::Waiting { .. }) || player.role != Role::Racer {
            let err = ServerMessage::error(ErrorCode::InvalidState, false);
            self.conns[&id].handle.send_reliable(&err.encode_frame());
            return;
        }
        if player.ready {
            return;
        }
        player.ready = true;
        room.revision += 1;
        room.broadcast(&ServerMessage::ReadyChanged {
            player_id: pid,
            ready: true,
            room_revision: room.revision,
        });
        self.try_schedule(room_id);
    }

    fn remove_player(&mut self, room_id: &str, pid: PlayerId, reason: LeaveReason) {
        let Some(room) = self.rooms.get_mut(room_id) else {
            return;
        };
        if room.players.remove(&pid).is_none() {
            return;
        }
        self.players -= 1;
        if let Race::Racing(run) | Race::Finished { run, .. } = &mut room.race
            && let Some(entry) = run.entry_mut(pid)
        {
            entry.present = false;
            if entry.status == CarStatus::Racing {
                entry.status = CarStatus::Dnf;
            }
        }
        if room.players.is_empty() {
            // Results and everything else of the room go with it.
            self.rooms.remove(room_id);
            return;
        }
        room.revision += 1;
        room.broadcast(&ServerMessage::PlayerLeft {
            player_id: pid,
            reason,
            room_revision: room.revision,
        });
        cancel_countdown(room);
        // The one who left may have been the last player not ready.
        self.try_schedule(room_id);
    }

    /// Reserves a start when enough racers are present and all are ready.
    fn try_schedule(&mut self, room_id: &str) {
        let room = self.rooms.get_mut(room_id).unwrap();
        let Race::Waiting { plan: plan @ None } = &mut room.race else {
            return;
        };
        let racers: Vec<PlayerId> = room
            .players
            .iter()
            .filter(|(_, p)| p.role == Role::Racer)
            .map(|(id, _)| *id)
            .collect();
        if racers.len() < self.cfg.min_racers
            || room
                .players
                .values()
                .any(|p| p.role == Role::Racer && !p.ready)
        {
            return;
        }
        let start = plan_start(&self.course, self.next_start_id, self.tick, &racers);
        self.next_start_id += 1;
        *plan = Some(start.clone());
        room.revision += 1;
        room.broadcast(&ServerMessage::RaceStart {
            start,
            room_revision: room.revision,
        });
    }

    /// One fixed tick for every room.
    pub fn tick(&mut self, now: Instant) {
        self.tick += 1;
        let tick = self.tick;
        self.keepalive(now);
        let course = &self.course;
        for room in self.rooms.values_mut() {
            collect_inputs(room, now);
            match &mut room.race {
                Race::Waiting { plan: Some(start) } if tick >= start.start_tick => {
                    let start = start.clone();
                    let players = &room.players;
                    room.race = Race::Racing(Run::new(start, |id| players[&id].name.clone()));
                },
                Race::Racing(run) => {
                    for entry in run.entries.iter_mut() {
                        if let Some(p) = room.players.get(&entry.id) {
                            entry.input = p.input;
                        }
                    }
                    if run.step(course, tick) {
                        let results = run.results(course);
                        let Race::Racing(run) =
                            std::mem::replace(&mut room.race, Race::Waiting { plan: None })
                        else {
                            unreachable!()
                        };
                        room.revision += 1;
                        room.broadcast(&ServerMessage::RaceFinished {
                            end_tick: tick,
                            results: results.clone(),
                            room_revision: room.revision,
                        });
                        room.race = Race::Finished { run, results };
                    }
                },
                _ => {},
            }
        }
    }

    /// Sends the latest state of each racing room to everyone in it.
    pub fn send_snapshots(&self) {
        for room in self.rooms.values() {
            let Race::Racing(run) = &room.race else {
                continue;
            };
            let mut snap = GameSnapshot {
                tick: self.tick,
                room_revision: room.revision,
                phase: RacePhase::Racing,
                start_id: run.start.start_id,
                start_tick: run.start.start_tick,
                entrants: run.entries.len() as u8,
                cars: run.cars(&self.course),
                last_processed_input: 0,
            };
            for p in room.players.values() {
                snap.last_processed_input = p.last_seq;
                let body = snap.encode();
                match fragment_snapshot(
                    snap.tick,
                    snap.room_revision,
                    &body,
                    p.conn.max_datagram_payload(),
                ) {
                    Some(datagrams) => p.conn.send_unreliable(datagrams),
                    None => {
                        let err = ServerMessage::error(ErrorCode::DatagramUnsupported, true);
                        p.conn.send_reliable(&err.encode_frame());
                        p.conn.close();
                    },
                }
            }
        }
    }

    /// Join deadline and application-level liveness via Ping/Pong.
    fn keepalive(&mut self, now: Instant) {
        let mut dead = Vec::new();
        for (id, conn) in self.conns.iter_mut() {
            match conn.joined {
                None if now - conn.since > JOIN_TIMEOUT => dead.push(*id),
                None => {},
                Some(_) => match conn.ping {
                    Some((_, sent)) if now - sent > PONG_TIMEOUT => dead.push(*id),
                    None if now - conn.last_ping >= PING_INTERVAL => {
                        let nonce = self.next_nonce;
                        self.next_nonce += 1;
                        conn.ping = Some((nonce, now));
                        conn.last_ping = now;
                        conn.handle
                            .send_reliable(&ServerMessage::Ping { nonce }.encode_frame());
                    },
                    _ => {},
                },
            }
        }
        for id in dead {
            self.reject(id, ErrorCode::Timeout);
        }
    }

    /// Tells everyone the server is stopping; nothing is saved.
    pub fn shutdown(&mut self) {
        let frame = ServerMessage::error(ErrorCode::ServerShutdown, true).encode_frame();
        for conn in self.conns.values() {
            conn.handle.send_reliable(&frame);
            conn.handle.close();
        }
        self.conns.clear();
        self.rooms.clear();
    }
}

/// Cancels a pending start, if any.
fn cancel_countdown(room: &mut Room) {
    let Race::Waiting { plan } = &mut room.race else {
        return;
    };
    let Some(start) = plan.take() else {
        return;
    };
    room.revision += 1;
    room.broadcast(&ServerMessage::RaceStartCancelled {
        start_id: start.start_id,
        reason: CancelReason::RosterChanged,
        room_revision: room.revision,
    });
}

/// Takes each player's newest input. Duplicates and reordered datagrams are
/// dropped; a silent player's controls go neutral after 250 ms.
fn collect_inputs(room: &mut Room, now: Instant) {
    for p in room.players.values_mut() {
        let newest = p
            .conn
            .take_inputs()
            .iter()
            .filter_map(|d| InputDatagram::decode(d).ok())
            .filter(|i| i.sequence > p.last_seq)
            .max_by_key(|i| i.sequence);
        if let Some(i) = newest {
            p.last_seq = i.sequence;
            p.input = to_input(i.controls);
            p.last_input = Some(now);
        }
        if p.last_input.is_none_or(|t| now - t > INPUT_TIMEOUT) {
            p.input = InputState::default();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pqpq_protocol::{Controls, FrameDecoder};
    use std::sync::Arc;

    fn config(min_racers: usize) -> Config {
        Config {
            native_addr: "127.0.0.1:0".parse().unwrap(),
            webtransport_addr: "127.0.0.1:0".parse().unwrap(),
            https_addr: "127.0.0.1:0".parse().unwrap(),
            web_root: "web".into(),
            web_origin: "https://localhost:8443".into(),
            tls_cert: "c".into(),
            tls_key: "k".into(),
            pin_web_certificate: false,
            max_connections: 4,
            max_rooms: 2,
            max_room_players: 3,
            min_racers,
        }
    }

    struct Client {
        handle: ConnectionHandle,
        decoder: FrameDecoder,
    }

    impl Client {
        fn connect(server: &mut Server, now: Instant) -> Client {
            let handle = ConnectionHandle::new(Arc::default());
            server.handle(Event::Connected(handle.clone()), now);
            Client {
                handle,
                decoder: FrameDecoder::default(),
            }
        }
        fn send(&self, server: &mut Server, msg: ClientMessage) {
            let frame = msg.encode_frame();
            server.handle(
                Event::Reliable(self.handle.id(), frame[4..].to_vec()),
                Instant::now(),
            );
        }
        fn join(server: &mut Server, name: &str, room: &str) -> Client {
            let c = Client::connect(server, Instant::now());
            c.send(
                server,
                ClientMessage::Join(Join {
                    protocol_version: PROTOCOL_VERSION,
                    username: name.into(),
                    room_id: room.into(),
                    course_id: COURSE_ID,
                    course_version: COURSE_VERSION,
                    physics_version: PHYSICS_VERSION,
                }),
            );
            c
        }
        fn received(&mut self) -> Vec<ServerMessage> {
            let out = self.handle.take_outgoing();
            self.decoder.push(&out.reliable);
            std::iter::from_fn(|| self.decoder.next_frame().unwrap())
                .map(|b| ServerMessage::decode(&b).unwrap())
                .collect()
        }
        fn joined(&mut self) -> Joined {
            self.received()
                .into_iter()
                .find_map(|m| {
                    if let ServerMessage::Joined(j) = m {
                        Some(j)
                    } else {
                        None
                    }
                })
                .expect("Joined")
        }
    }

    #[test]
    fn same_room_same_name_different_players() {
        let mut s = Server::new(config(2));
        let mut a = Client::join(&mut s, "foo", "1234");
        let ja = a.joined();
        let mut b = Client::join(&mut s, "foo", "1234");
        let jb = b.joined();
        assert_ne!(ja.player_id, jb.player_id);
        assert_eq!(jb.roster.len(), 2);
        assert_eq!(s.rooms.len(), 1);
        assert!(matches!(
            a.received()[..],
            [ServerMessage::PlayerJoined { .. }]
        ));
    }

    #[test]
    fn invalid_join_creates_no_room() {
        let mut s = Server::new(config(2));
        let mut c = Client::join(&mut s, "\u{1b}[2J", "1234");
        assert!(matches!(
            c.received()[..],
            [ServerMessage::Error {
                code: ErrorCode::InvalidUsername,
                fatal: true,
                ..
            }]
        ));
        assert!(c.handle.take_outgoing().close);
        assert!(s.rooms.is_empty());
        let mut c = Client::join(&mut s, "foo", "12 34");
        assert!(matches!(
            c.received()[..],
            [ServerMessage::Error {
                code: ErrorCode::InvalidRoomId,
                ..
            }]
        ));
        assert!(s.rooms.is_empty());
    }

    #[test]
    fn capacity_limits() {
        let mut s = Server::new(config(2));
        for _ in 0..3 {
            Client::join(&mut s, "a", "r1");
        }
        let mut full = Client::join(&mut s, "a", "r1");
        assert!(matches!(
            full.received()[..],
            [ServerMessage::Error {
                code: ErrorCode::RoomFull,
                ..
            }]
        ));
        Client::join(&mut s, "a", "r2");
        let mut over = Client::join(&mut s, "a", "r3");
        assert!(matches!(
            over.received()[..],
            [ServerMessage::Error {
                code: ErrorCode::ServerFull,
                ..
            }]
        ));
    }

    #[test]
    fn messages_before_join_and_second_join_are_violations() {
        let mut s = Server::new(config(2));
        let mut c = Client::connect(&mut s, Instant::now());
        c.send(&mut s, ClientMessage::Ready);
        assert!(matches!(
            c.received()[..],
            [ServerMessage::Error {
                code: ErrorCode::ProtocolViolation,
                ..
            }]
        ));
        let mut c = Client::join(&mut s, "a", "r");
        c.joined();
        c.send(
            &mut s,
            ClientMessage::Join(Join {
                protocol_version: PROTOCOL_VERSION,
                username: "a".into(),
                room_id: "r".into(),
                course_id: COURSE_ID,
                course_version: COURSE_VERSION,
                physics_version: PHYSICS_VERSION,
            }),
        );
        assert!(matches!(
            c.received()[..],
            [ServerMessage::Error {
                code: ErrorCode::ProtocolViolation,
                ..
            }]
        ));
        assert!(s.rooms.is_empty());
    }

    #[test]
    fn ready_countdown_cancel_and_start() {
        let mut s = Server::new(config(2));
        let mut a = Client::join(&mut s, "a", "r");
        let mut b = Client::join(&mut s, "b", "r");
        a.send(&mut s, ClientMessage::Ready);
        b.send(&mut s, ClientMessage::Ready);
        b.send(&mut s, ClientMessage::Ready); // duplicate is harmless
        let msgs = a.received();
        assert!(
            msgs.iter()
                .any(|m| matches!(m, ServerMessage::RaceStart { .. }))
        );
        // A newcomer cancels the countdown and is not ready.
        let mut c = Client::join(&mut s, "c", "r");
        assert!(
            a.received()
                .iter()
                .any(|m| matches!(m, ServerMessage::RaceStartCancelled { .. }))
        );
        assert!(c.joined().start.is_none());
        // Newcomer leaves: the others are still ready, so a new countdown starts.
        b.received();
        c.send(&mut s, ClientMessage::Leave);
        let msgs = b.received();
        let start = msgs.iter().find_map(|m| {
            if let ServerMessage::RaceStart { start, .. } = m {
                Some(start.clone())
            } else {
                None
            }
        });
        let start = start.expect("rescheduled");
        let now = Instant::now();
        while s.tick < start.start_tick {
            s.tick(now);
        }
        assert!(matches!(s.rooms["r"].race, Race::Racing(_)));
        // A late joiner spectates and Ready is refused.
        let mut d = Client::join(&mut s, "d", "r");
        assert_eq!(d.joined().role, Role::Spectator);
        d.send(&mut s, ClientMessage::Ready);
        assert!(matches!(
            d.received()[..],
            [ServerMessage::Error {
                code: ErrorCode::InvalidState,
                fatal: false,
                ..
            }]
        ));
    }

    #[test]
    fn inputs_are_newest_only_and_time_out() {
        let mut s = Server::new(config(1));
        let a = Client::join(&mut s, "a", "r");
        a.send(&mut s, ClientMessage::Ready);
        let now = Instant::now();
        while !matches!(s.rooms["r"].race, Race::Racing(_)) {
            s.tick(now);
        }
        let input = |seq, throttle| {
            InputDatagram {
                sequence: seq,
                controls: Controls {
                    throttle,
                    brake: false,
                    steering: 0,
                },
            }
            .encode()
        };
        a.handle.push_input(&input(5, true), now);
        a.handle.push_input(&input(3, false), now);
        s.tick(now);
        let p = &s.rooms["r"].players.values().next().unwrap();
        assert_eq!(p.last_seq, 5);
        assert!(p.input.throttle);
        // Old or repeated sequences change nothing.
        a.handle.push_input(&input(4, false), now);
        s.tick(now);
        assert!(s.rooms["r"].players.values().next().unwrap().input.throttle);
        s.tick(now + Duration::from_millis(300));
        assert!(!s.rooms["r"].players.values().next().unwrap().input.throttle);
    }

    #[test]
    fn disconnect_during_race_is_dnf_and_last_leave_deletes_room() {
        let mut s = Server::new(config(2));
        let a = Client::join(&mut s, "a", "r");
        let mut b = Client::join(&mut s, "b", "r");
        a.send(&mut s, ClientMessage::Ready);
        b.send(&mut s, ClientMessage::Ready);
        let now = Instant::now();
        while !matches!(s.rooms["r"].race, Race::Racing(_)) {
            s.tick(now);
        }
        s.handle(Event::Disconnected(a.handle.id()), now);
        s.tick(now);
        // b keeps racing alone.
        let Race::Racing(run) = &s.rooms["r"].race else {
            panic!()
        };
        assert_eq!(run.entries[0].status, CarStatus::Dnf);
        assert_eq!(run.cars(&s.course).len(), 1);
        b.received();
        s.handle(Event::Disconnected(b.handle.id()), now);
        assert!(s.rooms.is_empty());
        assert_eq!(s.counts(), (0, 0, 0));
    }

    #[test]
    fn join_timeout_and_ping_timeout() {
        let mut s = Server::new(config(2));
        let t0 = Instant::now();
        let mut idle = Client::connect(&mut s, t0);
        let mut a = Client::join(&mut s, "a", "r");
        s.tick(t0 + Duration::from_secs(6));
        assert!(matches!(
            idle.received()[..],
            [ServerMessage::Error {
                code: ErrorCode::Timeout,
                ..
            }]
        ));
        assert!(
            a.received()
                .iter()
                .any(|m| matches!(m, ServerMessage::Ping { .. }))
        );
        s.tick(t0 + Duration::from_secs(12));
        assert!(a.received().iter().any(|m| matches!(
            m,
            ServerMessage::Error {
                code: ErrorCode::Timeout,
                ..
            }
        )));
        assert!(s.rooms.is_empty());
    }
}
