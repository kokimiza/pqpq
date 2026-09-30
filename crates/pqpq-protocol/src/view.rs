//! Client-side room state built from control messages, and the ordering rule
//! between the reliable stream and unreliable snapshots (room_revision).

use crate::{
    GameSnapshot, Joined, MAX_PENDING_SNAPSHOTS, PlayerId, RacePhase, RaceStart, ResultEntry, Role,
    RosterEntry, ServerMessage,
};

#[derive(Clone, Debug)]
pub struct RoomView {
    pub me: PlayerId,
    pub room_id: String,
    pub role: Role,
    /// Connected players, sorted by id.
    pub roster: Vec<RosterEntry>,
    pub revision: u64,
    pub phase: RacePhase,
    pub start: Option<RaceStart>,
    pub results: Vec<ResultEntry>,
    pub laps: u8,
    pub min_racers: u8,
    /// Snapshots whose control messages have not arrived yet.
    pending: Vec<GameSnapshot>,
    last_tick: u64,
}

impl RoomView {
    pub fn new(joined: Joined) -> Self {
        let mut roster = joined.roster;
        roster.sort_by_key(|e| e.player_id);
        RoomView {
            me: joined.player_id,
            room_id: joined.room_id,
            role: joined.role,
            roster,
            revision: joined.room_revision,
            phase: joined.phase,
            start: joined.start,
            results: joined.results,
            laps: joined.laps,
            min_racers: joined.min_racers,
            pending: Vec::new(),
            last_tick: joined.server_tick,
        }
    }

    /// Applies one control message and returns snapshots it unblocked.
    pub fn apply(&mut self, msg: &ServerMessage) -> Vec<GameSnapshot> {
        match msg {
            ServerMessage::PlayerJoined { entry, .. } => {
                self.roster.retain(|e| e.player_id != entry.player_id);
                self.roster.push(entry.clone());
                self.roster.sort_by_key(|e| e.player_id);
            },
            ServerMessage::PlayerLeft { player_id, .. } => {
                self.roster.retain(|e| e.player_id != *player_id)
            },
            ServerMessage::ReadyChanged {
                player_id, ready, ..
            } => {
                if let Some(e) = self.roster.iter_mut().find(|e| e.player_id == *player_id) {
                    e.ready = *ready;
                }
            },
            ServerMessage::RaceStart { start, .. } => self.start = Some(start.clone()),
            ServerMessage::RaceStartCancelled { start_id, .. } => {
                if self.phase == RacePhase::Waiting
                    && self.start.as_ref().is_some_and(|s| s.start_id == *start_id)
                {
                    self.start = None;
                }
            },
            ServerMessage::RaceFinished { results, .. } => {
                self.phase = RacePhase::Finished;
                self.results = results.clone();
                self.pending.clear();
            },
            _ => {},
        }
        if let Some(rev) = msg.room_revision() {
            self.revision = self.revision.max(rev);
        }
        let mut ready: Vec<GameSnapshot> = Vec::new();
        let pending = std::mem::take(&mut self.pending);
        for snap in pending {
            if snap.room_revision > self.revision {
                self.pending.push(snap);
            } else if let Some(s) = self.offer(snap) {
                ready.push(s);
            }
        }
        ready
    }

    /// Returns the snapshot when it is usable now; holds it if it is ahead of
    /// the control stream; drops it if it is stale.
    pub fn offer(&mut self, snap: GameSnapshot) -> Option<GameSnapshot> {
        if self.phase == RacePhase::Finished
            || snap.tick <= self.last_tick
            || snap.room_revision < self.revision
        {
            return None;
        }
        if snap.room_revision > self.revision {
            if self.pending.len() >= MAX_PENDING_SNAPSHOTS {
                let oldest = (0..self.pending.len())
                    .min_by_key(|&i| self.pending[i].tick)
                    .unwrap();
                self.pending.remove(oldest);
            }
            self.pending.push(snap);
            self.pending.sort_by_key(|s| s.tick);
            return None;
        }
        self.last_tick = snap.tick;
        if snap.phase == RacePhase::Racing {
            self.phase = RacePhase::Racing;
        }
        Some(snap)
    }

    pub fn name_of(&self, id: PlayerId) -> Option<&str> {
        self.roster
            .iter()
            .find(|e| e.player_id == id)
            .map(|e| e.username.as_str())
            .or_else(|| {
                self.results
                    .iter()
                    .find(|r| r.player_id == id)
                    .map(|r| r.username.as_str())
            })
    }

    pub fn label_index(&self, id: PlayerId) -> Option<usize> {
        self.start
            .as_ref()
            .and_then(|s| s.grid.iter().position(|g| g.player_id == id))
    }

    /// A..Z, AA..AZ, ...; also unique in rooms with more than 26 racers.
    pub fn label_of(&self, id: PlayerId) -> String {
        let Some(mut index) = self.label_index(id) else {
            return "?".into();
        };
        let mut label = Vec::new();
        loop {
            label.push((b'A' + (index % 26) as u8) as char);
            if index < 26 {
                break;
            }
            index = index / 26 - 1;
        }
        label.into_iter().rev().collect()
    }

    pub fn racers(&self) -> impl Iterator<Item = &RosterEntry> {
        self.roster.iter().filter(|e| e.role == Role::Racer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CancelReason, LeaveReason};

    fn joined() -> Joined {
        Joined {
            player_id: PlayerId(1),
            room_id: "r".into(),
            role: Role::Racer,
            roster: vec![RosterEntry {
                player_id: PlayerId(1),
                username: "a".into(),
                role: Role::Racer,
                ready: false,
            }],
            room_revision: 1,
            phase: RacePhase::Waiting,
            server_tick: 10,
            start: None,
            course_id: 1,
            course_version: 1,
            physics_version: 1,
            tick_hz: 30,
            laps: 3,
            min_racers: 2,
            results: vec![],
        }
    }

    fn snap(tick: u64, rev: u64) -> GameSnapshot {
        GameSnapshot {
            tick,
            room_revision: rev,
            phase: RacePhase::Racing,
            start_id: 1,
            start_tick: 0,
            entrants: 1,
            cars: vec![],
            last_processed_input: 0,
        }
    }

    #[test]
    fn snapshot_ahead_of_stream_waits_for_control_message() {
        let mut v = RoomView::new(joined());
        assert!(v.offer(snap(20, 2)).is_none());
        let released = v.apply(&ServerMessage::PlayerLeft {
            player_id: PlayerId(9),
            reason: LeaveReason::Left,
            room_revision: 2,
        });
        assert_eq!(released.len(), 1);
        assert_eq!(v.phase, RacePhase::Racing);
    }

    #[test]
    fn old_and_repeated_snapshots_are_dropped() {
        let mut v = RoomView::new(joined());
        assert!(v.offer(snap(5, 1)).is_none()); // before Joined's tick
        assert!(v.offer(snap(20, 1)).is_some());
        assert!(v.offer(snap(20, 1)).is_none());
        assert!(v.offer(snap(19, 1)).is_none());
        v.apply(&ServerMessage::ReadyChanged {
            player_id: PlayerId(1),
            ready: true,
            room_revision: 2,
        });
        assert!(v.offer(snap(21, 1)).is_none()); // older revision
    }

    #[test]
    fn nothing_moves_after_finish() {
        let mut v = RoomView::new(joined());
        v.apply(&ServerMessage::RaceFinished {
            end_tick: 30,
            results: vec![],
            room_revision: 2,
        });
        assert!(v.offer(snap(40, 2)).is_none());
        assert_eq!(v.phase, RacePhase::Finished);
    }

    #[test]
    fn labels_stay_unique_beyond_twenty_six_racers() {
        let mut v = RoomView::new(joined());
        v.start = Some(RaceStart {
            start_id: 1,
            start_tick: 100,
            server_tick: 10,
            grid: (1..=255)
                .map(|id| crate::GridSlot {
                    player_id: PlayerId(id),
                    position: [0.0; 2],
                    direction: 0.0,
                })
                .collect(),
        });
        let labels: std::collections::HashSet<_> =
            (1..=255).map(|id| v.label_of(PlayerId(id))).collect();
        assert_eq!(labels.len(), 255);
        assert_eq!(v.label_of(PlayerId(26)), "Z");
        assert_eq!(v.label_of(PlayerId(27)), "AA");
    }

    #[test]
    fn cancel_clears_only_the_matching_start() {
        let mut v = RoomView::new(joined());
        let start = RaceStart {
            start_id: 3,
            start_tick: 100,
            server_tick: 10,
            grid: vec![],
        };
        v.apply(&ServerMessage::RaceStart {
            start,
            room_revision: 2,
        });
        v.apply(&ServerMessage::RaceStartCancelled {
            start_id: 2,
            reason: CancelReason::RosterChanged,
            room_revision: 3,
        });
        assert!(v.start.is_some());
        v.apply(&ServerMessage::RaceStartCancelled {
            start_id: 3,
            reason: CancelReason::RosterChanged,
            room_revision: 4,
        });
        assert!(v.start.is_none());
    }
}
