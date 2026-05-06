//! In-process event hub backing the `/events` SSE stream.
//!
//! Per the web-edit-sync design doc §5.5: a single mutex covers seq
//! advancement and the replay ring so seq order matches publish order.
//! `subscribe_since` returns a `ReplayStream` that subscribes to the
//! broadcast channel *before* snapshotting the ring under the same lock,
//! so events landing during the handoff arrive via the live stream and
//! are de-duplicated by `drop_through` at the SSE handler.

use std::collections::VecDeque;

use parking_lot::Mutex;
use serde::Serialize;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::repo::{IssueSummary, LoadWarning};

/// Default ring capacity. 1024 covers a normal `git checkout` plus normal
/// editor activity; bulk operations beyond that trip the §5.7 coalescer.
pub const DEFAULT_RING_CAPACITY: usize = 1024;
/// Default broadcast channel capacity. Slow consumers that fall behind
/// this many events get a `Lagged` error — the SSE handler maps that to
/// a `Resync { reason: "lagged" }`.
pub const DEFAULT_BROADCAST_CAPACITY: usize = 1024;

#[derive(Debug, Clone, Serialize)]
pub struct BoardEvent {
    pub seq: u64,
    #[serde(flatten)]
    pub payload: EventPayload,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum EventPayload {
    IssueUpserted {
        slug: String,
        version: String,
        issue: Box<IssueSummary>,
    },
    IssueRemoved {
        slug: String,
    },
    IssueInvalid {
        slug: String,
        warnings: Vec<LoadWarning>,
    },
    Resync {
        reason: String,
    },
    Degraded {
        reason: String,
    },
}

pub struct EventHub {
    inner: Mutex<EventHubInner>,
    tx: broadcast::Sender<BoardEvent>,
    capacity: usize,
    instance_id: Uuid,
}

struct EventHubInner {
    next_seq: u64,
    ring: VecDeque<BoardEvent>,
}

impl EventHub {
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_RING_CAPACITY, DEFAULT_BROADCAST_CAPACITY)
    }

    pub fn with_capacity(ring_capacity: usize, broadcast_capacity: usize) -> Self {
        assert!(ring_capacity > 0, "ring capacity must be > 0");
        assert!(broadcast_capacity > 0, "broadcast capacity must be > 0");
        let (tx, _) = broadcast::channel(broadcast_capacity);
        EventHub {
            inner: Mutex::new(EventHubInner {
                next_seq: 0,
                ring: VecDeque::with_capacity(ring_capacity),
            }),
            tx,
            capacity: ring_capacity,
            instance_id: Uuid::new_v4(),
        }
    }

    pub fn instance_id(&self) -> Uuid {
        self.instance_id
    }

    /// Snapshot of the highest seq the ring contains. Use as the
    /// `snapshot_seq` cursor returned by `/api/issues` — taken *before*
    /// any filesystem scan to close the lost-event window.
    pub fn current_seq(&self) -> u64 {
        self.inner.lock().next_seq
    }

    /// Allocate a seq, push to ring, and broadcast. Returns the published
    /// event so callers can log/inspect. Callers in `mutate.rs` (M1) must
    /// invoke this *before* releasing the repo `flock`.
    pub fn publish(&self, payload: EventPayload) -> BoardEvent {
        let evt = {
            let mut g = self.inner.lock();
            g.next_seq += 1;
            let evt = BoardEvent {
                seq: g.next_seq,
                payload,
            };
            while g.ring.len() >= self.capacity {
                g.ring.pop_front();
            }
            g.ring.push_back(evt.clone());
            evt
        };
        // Drop lock before send so a slow subscriber can't stall publishers.
        let _ = self.tx.send(evt.clone());
        evt
    }

    /// Subscribe to the broadcast channel only, without snapshotting the
    /// replay ring. Test-only helper: production code uses
    /// `subscribe_since` so the subscriber sees both replay and live
    /// events with the race-free handoff.
    #[cfg(test)]
    pub fn tx_subscribe_for_test(&self) -> broadcast::Receiver<BoardEvent> {
        self.tx.subscribe()
    }

    /// Subscribe to live events and snapshot the replay ring atomically
    /// w.r.t. concurrent `publish` calls. The caller forwards `replay`
    /// events first, then forwards `rx` events with `seq > drop_through`.
    pub fn subscribe_since(&self, since: u64) -> ReplayStream {
        // Subscribe FIRST so any event published after we drop the lock
        // is captured by `rx` rather than lost between snapshot and
        // subscribe.
        let rx = self.tx.subscribe();
        let g = self.inner.lock();
        let current = g.next_seq;
        let replay = if since > current {
            // Future seq → previous server instance, or stale client.
            // `instance_id` mismatch should also drive resync independently.
            Replay::TooOld {
                reason: "future_seq",
            }
        } else if since == current {
            Replay::Events(Vec::new())
        } else if g.ring.is_empty() {
            // Nothing buffered but `since < current` — gap.
            Replay::TooOld { reason: "gap" }
        } else {
            // `since == oldest - 1` is replayable: oldest is the first
            // event after the gap, so contents from oldest .. current
            // cover (since, current].
            let oldest_seq = g.ring.front().unwrap().seq;
            if oldest_seq > since + 1 {
                Replay::TooOld { reason: "gap" }
            } else {
                let evts: Vec<BoardEvent> =
                    g.ring.iter().filter(|e| e.seq > since).cloned().collect();
                Replay::Events(evts)
            }
        };
        let drop_through = match &replay {
            Replay::Events(v) => v.last().map(|e| e.seq).unwrap_or(since),
            Replay::TooOld { .. } => current,
        };
        ReplayStream {
            replay,
            rx,
            drop_through,
        }
    }
}

impl Default for EventHub {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
pub enum Replay {
    Events(Vec<BoardEvent>),
    TooOld { reason: &'static str },
}

pub struct ReplayStream {
    pub replay: Replay,
    pub rx: broadcast::Receiver<BoardEvent>,
    pub drop_through: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_payload(slug: &str) -> EventPayload {
        EventPayload::IssueRemoved {
            slug: slug.to_string(),
        }
    }

    #[test]
    fn seq_advances_in_publish_order() {
        let hub = EventHub::new();
        assert_eq!(hub.current_seq(), 0);
        let a = hub.publish(dummy_payload("a"));
        let b = hub.publish(dummy_payload("b"));
        assert_eq!(a.seq, 1);
        assert_eq!(b.seq, 2);
        assert_eq!(hub.current_seq(), 2);
    }

    #[test]
    fn subscribe_since_returns_empty_when_caught_up() {
        let hub = EventHub::new();
        hub.publish(dummy_payload("a"));
        let s = hub.subscribe_since(1);
        match s.replay {
            Replay::Events(v) => assert!(v.is_empty()),
            Replay::TooOld { .. } => panic!("expected caught-up"),
        }
        assert_eq!(s.drop_through, 1);
    }

    #[test]
    fn subscribe_since_replays_recent_events() {
        let hub = EventHub::new();
        hub.publish(dummy_payload("a")); // seq=1
        hub.publish(dummy_payload("b")); // seq=2
        hub.publish(dummy_payload("c")); // seq=3
        let s = hub.subscribe_since(1);
        let evts = match s.replay {
            Replay::Events(v) => v,
            Replay::TooOld { .. } => panic!(),
        };
        assert_eq!(evts.len(), 2);
        assert_eq!(evts[0].seq, 2);
        assert_eq!(evts[1].seq, 3);
        assert_eq!(s.drop_through, 3);
    }

    #[test]
    fn subscribe_since_future_seq_yields_too_old() {
        let hub = EventHub::new();
        hub.publish(dummy_payload("a"));
        let s = hub.subscribe_since(99);
        match s.replay {
            Replay::TooOld { reason } => assert_eq!(reason, "future_seq"),
            Replay::Events(_) => panic!("expected TooOld"),
        }
    }

    #[test]
    fn subscribe_since_gap_yields_too_old() {
        let hub = EventHub::with_capacity(2, 16);
        hub.publish(dummy_payload("a")); // 1
        hub.publish(dummy_payload("b")); // 2
        hub.publish(dummy_payload("c")); // 3 -- evicts seq=1
                                         // ring now [seq=2, seq=3]. oldest=2.
                                         // since=0: oldest > since+1 (2 > 1) → gap.
        let s = hub.subscribe_since(0);
        match s.replay {
            Replay::TooOld { reason } => assert_eq!(reason, "gap"),
            Replay::Events(_) => panic!("expected gap"),
        }
        // since=1: oldest == since+1 → replayable (seq=2,3).
        let s = hub.subscribe_since(1);
        match s.replay {
            Replay::Events(v) => {
                assert_eq!(v.len(), 2);
                assert_eq!(v[0].seq, 2);
            }
            Replay::TooOld { .. } => panic!("seq=since+1 must replay"),
        }
    }

    #[test]
    fn published_events_reach_subscribers() {
        let hub = EventHub::new();
        let mut s = hub.subscribe_since(0);
        let evt = hub.publish(dummy_payload("a"));
        // The broadcast send is non-blocking; receiver is a sync API on
        // tokio::sync::broadcast::Receiver — use try_recv.
        let recv = s.rx.try_recv().unwrap();
        assert_eq!(recv.seq, evt.seq);
    }

    #[test]
    fn instance_id_stable_across_calls() {
        let hub = EventHub::new();
        let a = hub.instance_id();
        let b = hub.instance_id();
        assert_eq!(a, b);
    }
}
