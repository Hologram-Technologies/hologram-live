//! The set of peers this node contacts, and when.
//!
//! A rotating cursor gives every peer a turn, so a large cluster cannot starve
//! the peers whose origins sort late. Failures back off; configured seeds are
//! the recovery path and are never dropped.

use super::normalize_endpoint;
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

    /// Recovers the working set from the persisted directory. Called once at
    /// startup, and — since `run` (`src/cluster/mod.rs`) now re-seeds every
    /// heartbeat round to let a seed notice and dial a joiner back — again on
    /// every round after that.
    ///
    /// `self_endpoint` is excluded: the directory contains this node's own
    /// heartbeat record (`NodeDirectory::heartbeat` persists self), and
    /// without this filter a restarted node would add itself as a peer and
    /// burn one `fanout` slot every round, forever (it is never pruned,
    /// since `prune_older_than` always preserves the local node id).
    ///
    /// Endpoints are normalized before comparison and insertion, matching the
    /// gossiped-peer path (`run`'s `for peer in response.peers` handling) and
    /// `prune_evictions`. Without this, a directory entry written with a
    /// trailing slash (or another spelling `normalize_endpoint` collapses)
    /// would be stored under a table key that `evict` — always called with a
    /// normalized endpoint — can never match, making that entry permanently
    /// un-evictable and burning a fanout slot forever, the same bug class
    /// Task 6 fixed for the self-seeding case.
    ///
    /// `max_peers` bounds growth the same way the gossiped-peer path already
    /// does: `NodeDirectory` is an unbounded map fed by every inbound join,
    /// so without a cap here a re-seed on every round could grow the table
    /// past the configured bound even though `fanout` still caps how many of
    /// its entries are dialed per round.
    pub fn seed_from_directory(
        &mut self,
        nodes: &[NodeRecord],
        self_endpoint: &str,
        max_peers: usize,
    ) {
        for node in nodes {
            if self.len() >= max_peers {
                break;
            }
            if node.endpoint.is_empty() {
                continue;
            }
            let endpoint = normalize_endpoint(&node.endpoint);
            if endpoint != self_endpoint {
                self.insert(endpoint);
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
            8,
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
            8,
        );
        assert_eq!(table.due(0, 8), vec!["https://known.example".to_owned()]);
    }

    // Fix round 3, finding 3: the gossiped-peer path in `run` normalizes
    // before inserting or comparing, and `prune_evictions` normalizes before
    // calling `evict`. Before this fix, `seed_from_directory` inserted the
    // raw directory endpoint, so a peer advertising a trailing slash got a
    // table key `evict`'s normalized argument could never match — a
    // permanently un-evictable entry burning a fanout slot forever, the same
    // bug class Task 6 fixed for the self-seeding case.
    #[test]
    fn seed_from_directory_normalizes_a_trailing_slash_endpoint_so_it_stays_evictable() {
        let mut table = PeerTable::new(Vec::new());
        table.seed_from_directory(
            &[crate::protocol::NodeRecord {
                node_id: "ed25519:peer".to_owned(),
                version: "test".to_owned(),
                operations: Vec::new(),
                endpoint: "https://known.example/".to_owned(),
                last_seen_millis: 0,
            }],
            "https://self.example",
            8,
        );
        // The unnormalized key would never match this call, and the peer
        // would remain forever.
        table.evict("https://known.example");
        assert!(
            table.due(0, 8).is_empty(),
            "a trailing-slash directory endpoint must still be evictable once normalized"
        );
    }

    // Fix round 3, finding 2: the gossiped-peer path already stops inserting
    // once `table.len() >= config.max_peers`; `seed_from_directory` re-runs
    // every round now (fix round 2), reading from an unbounded
    // `NodeDirectory`, so without the same cap it could grow the table past
    // the configured bound even though `fanout` still caps dials per round.
    #[test]
    fn seed_from_directory_stops_growing_the_table_past_max_peers() {
        let mut table = PeerTable::new(Vec::new());
        let nodes: Vec<crate::protocol::NodeRecord> = (0..10)
            .map(|index| crate::protocol::NodeRecord {
                node_id: format!("ed25519:{index:02}"),
                version: "test".to_owned(),
                operations: Vec::new(),
                endpoint: format!("https://node{index:02}.example"),
                last_seen_millis: 0,
            })
            .collect();
        table.seed_from_directory(&nodes, "https://self.example", 3);
        assert_eq!(
            table.len(),
            3,
            "seed_from_directory must respect max_peers even though the directory offered more"
        );
    }

    // Fix round 3, finding 1: `run` now calls `seed_from_directory` every
    // heartbeat round, not just once at startup, so a peer that is still
    // failing (and still present in the directory, since it has not yet
    // aged out) must not have its backoff reset by the next re-seed —
    // `insert`'s `entry().or_insert(..)` leaves an existing entry alone, but
    // nothing previously asserted that directly.
    #[test]
    fn reseeding_from_the_directory_does_not_reset_an_in_progress_backoff() {
        let mut table = PeerTable::new(Vec::new());
        table.insert("https://flaky.example".to_owned());
        table.record_failure("https://flaky.example", 0, 60_000);
        assert!(
            table.due(1_000, 8).is_empty(),
            "the peer is backing off before any re-seed"
        );

        // Simulate the periodic re-seed `run` performs every round: the same
        // peer is still in the directory (it has not aged out).
        table.seed_from_directory(
            &[crate::protocol::NodeRecord {
                node_id: "ed25519:flaky".to_owned(),
                version: "test".to_owned(),
                operations: Vec::new(),
                endpoint: "https://flaky.example".to_owned(),
                last_seen_millis: 0,
            }],
            "https://self.example",
            8,
        );

        assert!(
            table.due(1_000, 8).is_empty(),
            "re-seeding an already-known, still-backing-off peer must not clear its backoff"
        );
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
