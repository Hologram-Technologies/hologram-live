//! The set of peers this node contacts, and when.
//!
//! A rotating cursor gives every peer a turn, so a large cluster cannot starve
//! the peers whose origins sort late. Failures back off; configured seeds are
//! the recovery path and are never dropped.

use crate::protocol::NodeRecord;
use std::collections::BTreeMap;

const BASE_BACKOFF_MILLIS: u64 = 15_000;

struct PeerState {
    is_seed: bool,
    failures: u32,
    next_attempt_millis: u64,
}

pub struct PeerTable {
    peers: BTreeMap<String, PeerState>,
    cursor: usize,
}

impl PeerTable {
    pub fn new(seeds: Vec<String>) -> Self {
        let mut table = Self {
            peers: BTreeMap::new(),
            cursor: 0,
        };
        for seed in seeds {
            table.peers.insert(
                seed,
                PeerState {
                    is_seed: true,
                    failures: 0,
                    next_attempt_millis: 0,
                },
            );
        }
        table
    }

    /// Recovers the working set from the persisted directory on restart.
    ///
    /// `self_endpoint` is excluded: the directory contains this node's own
    /// heartbeat record (`NodeDirectory::heartbeat` persists self), and
    /// without this filter a restarted node would add itself as a peer and
    /// burn one `fanout` slot every round, forever (it is never pruned,
    /// since `prune_older_than` always preserves the local node id).
    pub fn seed_from_directory(&mut self, nodes: &[NodeRecord], self_endpoint: &str) {
        for node in nodes {
            if !node.endpoint.is_empty() && node.endpoint != self_endpoint {
                self.insert(node.endpoint.clone());
            }
        }
    }

    pub fn insert(&mut self, endpoint: String) {
        self.peers.entry(endpoint).or_insert(PeerState {
            is_seed: false,
            failures: 0,
            next_attempt_millis: 0,
        });
    }

    pub fn len(&self) -> usize {
        self.peers.len()
    }

    /// The next `fanout` peers whose backoff has elapsed, advancing the cursor
    /// so the following round starts where this one stopped.
    pub fn due(&mut self, now_millis: u64, fanout: usize) -> Vec<String> {
        if self.peers.is_empty() || fanout == 0 {
            return Vec::new();
        }
        let ordered: Vec<String> = self.peers.keys().cloned().collect();
        let start = self.cursor % ordered.len();
        let mut due = Vec::new();
        for offset in 0..ordered.len() {
            if due.len() >= fanout {
                break;
            }
            let endpoint = &ordered[(start + offset) % ordered.len()];
            if self.peers[endpoint].next_attempt_millis <= now_millis {
                due.push(endpoint.clone());
            }
        }
        self.cursor = (start + ordered.len().min(fanout.max(1))) % ordered.len();
        due
    }

    pub fn record_success(&mut self, endpoint: &str, _now_millis: u64) {
        if let Some(state) = self.peers.get_mut(endpoint) {
            state.failures = 0;
            state.next_attempt_millis = 0;
        }
    }

    pub fn record_failure(&mut self, endpoint: &str, now_millis: u64, ceiling_millis: u64) {
        if let Some(state) = self.peers.get_mut(endpoint) {
            state.failures = state.failures.saturating_add(1);
            let delay = BASE_BACKOFF_MILLIS
                .saturating_mul(1_u64 << state.failures.min(12))
                .min(ceiling_millis.max(BASE_BACKOFF_MILLIS));
            state.next_attempt_millis = now_millis.saturating_add(delay);
        }
    }

    /// Drops a peer unless it is a configured seed.
    pub fn evict(&mut self, endpoint: &str) {
        if self.peers.get(endpoint).is_some_and(|state| !state.is_seed) {
            self.peers.remove(endpoint);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn endpoints(count: usize) -> Vec<String> {
        (0..count)
            .map(|i| format!("https://node{i:02}.example"))
            .collect()
    }

    // Defect 7: the old BTreeSet + take(n) reached only the alphabetically first n.
    #[test]
    fn the_cursor_reaches_every_peer() {
        let mut table = PeerTable::new(Vec::new());
        for endpoint in endpoints(20) {
            table.insert(endpoint);
        }
        let mut seen = std::collections::BTreeSet::new();
        let mut now = 0;
        for _ in 0..(20_usize.div_ceil(4)) {
            let batch = table.due(now, 4);
            // Fix round 1, finding 3: `record_success` clears backoff, so
            // every peer is due every round regardless of `fanout` — without
            // this bound, an implementation that ignored `fanout` entirely
            // and returned every due peer would still pass on round one.
            assert!(
                batch.len() <= 4,
                "due() must never return more than fanout entries, got {}",
                batch.len()
            );
            for endpoint in batch {
                seen.insert(endpoint.clone());
                table.record_success(&endpoint, now);
            }
            now += 15_000;
        }
        assert_eq!(
            seen.len(),
            20,
            "every peer must be contacted within n/fanout rounds"
        );
    }

    // Fix round 1, finding 3: a direct case with more due peers than fanout,
    // so the cap is pinned even without relying on the rotation test above.
    #[test]
    fn due_never_exceeds_the_requested_fanout() {
        let mut table = PeerTable::new(Vec::new());
        for endpoint in endpoints(10) {
            table.insert(endpoint);
        }
        assert_eq!(
            table.due(0, 3).len(),
            3,
            "ten peers are due and fanout is 3, so due() must return exactly 3"
        );
    }

    #[test]
    fn a_failing_peer_backs_off_and_is_capped() {
        let mut table = PeerTable::new(Vec::new());
        table.insert("https://dead.example".to_owned());
        assert_eq!(table.due(0, 8).len(), 1);
        table.record_failure("https://dead.example", 0, 60_000);
        assert!(table.due(1_000, 8).is_empty(), "a failed peer waits");
        for attempt in 1..10 {
            table.record_failure("https://dead.example", attempt * 1_000, 60_000);
        }
        assert!(
            table.due(9_000 + 60_001, 8).len() == 1,
            "backoff must be capped, not unbounded"
        );
    }

    // Defect 8: a restart with no configured seeds must not be isolated.
    #[test]
    fn the_table_recovers_from_the_persisted_directory() {
        let mut table = PeerTable::new(Vec::new());
        table.seed_from_directory(
            &[crate::protocol::NodeRecord {
                node_id: "ed25519:aa".to_owned(),
                version: "test".to_owned(),
                operations: Vec::new(),
                endpoint: "https://known.example".to_owned(),
                last_seen_millis: 0,
            }],
            "https://self.example",
        );
        assert_eq!(table.due(0, 8), vec!["https://known.example".to_owned()]);
    }

    // Fix round 1, finding 1: `NodeDirectory::heartbeat` persists this node's
    // own record, so the directory handed to `seed_from_directory` always
    // contains self on every restart after the first cold start. Without the
    // filter, self becomes an un-evictable peer (it is always preserved by
    // `prune_older_than`) and permanently burns a fanout slot.
    #[test]
    fn seeding_from_the_directory_never_adds_this_node_as_its_own_peer() {
        let mut table = PeerTable::new(Vec::new());
        table.seed_from_directory(
            &[
                crate::protocol::NodeRecord {
                    node_id: "ed25519:self".to_owned(),
                    version: "test".to_owned(),
                    operations: Vec::new(),
                    endpoint: "https://self.example".to_owned(),
                    last_seen_millis: 0,
                },
                crate::protocol::NodeRecord {
                    node_id: "ed25519:peer".to_owned(),
                    version: "test".to_owned(),
                    operations: Vec::new(),
                    endpoint: "https://known.example".to_owned(),
                    last_seen_millis: 0,
                },
            ],
            "https://self.example",
        );
        assert_eq!(table.due(0, 8), vec!["https://known.example".to_owned()]);
    }

    #[test]
    fn a_configured_seed_is_never_evicted() {
        let mut table = PeerTable::new(vec!["https://seed.example".to_owned()]);
        table.insert("https://learned.example".to_owned());
        table.evict("https://seed.example");
        table.evict("https://learned.example");
        assert_eq!(table.due(0, 8), vec!["https://seed.example".to_owned()]);
    }

    // Review Focus 4: no seeds, empty directory, nothing to do.
    #[test]
    fn an_empty_table_is_idle_rather_than_hot() {
        let mut table = PeerTable::new(Vec::new());
        assert!(table.due(0, 8).is_empty());
        assert!(table.due(0, 0).is_empty());
        assert_eq!(table.len(), 0);
    }
}
