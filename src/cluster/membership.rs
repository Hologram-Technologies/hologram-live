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

    pub fn seed_from_directory(&mut self, nodes: &[NodeRecord]) {
        for node in nodes {
            if !node.endpoint.is_empty() {
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
            for endpoint in table.due(now, 4) {
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
        table.seed_from_directory(&[crate::protocol::NodeRecord {
            node_id: "ed25519:aa".to_owned(),
            version: "test".to_owned(),
            operations: Vec::new(),
            endpoint: "https://known.example".to_owned(),
            last_seen_millis: 0,
        }]);
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
