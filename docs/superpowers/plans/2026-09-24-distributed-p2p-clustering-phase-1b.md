# Distributed P2P Clustering — Phase 1 Implementation Plan (Tasks 6–9)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Hologram cluster membership actually work between distinct hosts by giving every node a per-node ed25519 identity, authenticating every cluster request against it, and fixing the peer-set, restart, and replication defects — all still over HTTP.

**Architecture:** `src/cluster.rs` becomes a `src/cluster/` module directory. A persistent ed25519 keypair at `<state_dir>/node.key` replaces the derived `server_id`. Every cluster request carries an ed25519 proof over a canonical preimage that binds method, path, query, recipient, timestamp, and body digest. An `Admission` trait decides who may participate, with a static allowlist and a trust-on-first-use implementation over the existing shared token. A peer table with backoff, eviction, and rotating-cursor fanout replaces the append-only `BTreeSet`.

**Tech Stack:** Rust 2021 (rust-version 1.95), tokio, axum, reqwest, blake3, `ed25519-dalek` 3.0 (new direct dependency; already resolved in `Cargo.lock` as a transitive, so adding it does not move the lock).

**Spec:** `docs/superpowers/specs/2026-09-24-distributed-p2p-clustering-design.md`

**Worktree:** `/private/tmp/hologram-cluster-p2p`, branch `cluster/distributed-p2p-design`.

**Phase 2 (the iroh transport) is deliberately not in this plan.** It depends on the interfaces Tasks 2–6 produce and on `iroh-blobs` 0.x APIs. It gets its own plan once Phase 1 lands.

## Global Constraints

- Rust `rust-version = "1.95"`; do not lower it.
- `cargo clippy --workspace --all-targets --locked -- -D warnings` must pass. No `#[allow]` without a comment explaining why.
- `./scripts/check-file-size.sh`: 1500 production lines per file, counted up to the first `#[cfg(test)]`. Every new file must stay well under it.
- `./scripts/check-kappa-pin.sh`: `topcoat`, `veilid`, `openssl-sys`, `aws-lc`, `rekindle` must not appear in `cargo tree --package hologram-live --features oci --edges normal`.
- `deny.toml` allows `MIT`, `Apache-2.0`, `BSD-2-Clause`, `BSD-3-Clause`, `ISC`, `Unicode-3.0`, `Zlib`, `CC0-1.0`, `BSL-1.0`. `ed25519-dalek` is BSD-3-Clause and needs no exception.
- `ClusterConfig` carries `#[serde(default, deny_unknown_fields)]`. Config changes must be **additive only**; removing or renaming a field breaks existing `live.toml` files.
- Any new dependency must be recorded in `DEPENDENCIES.md`.
- Tests run serialized: `cargo test --workspace --all-targets --locked -- --test-threads=1`.
- Never log, or include in an error message, the contents of `node.key` or the cluster token.

## Review Focus

Five conditions the spec implies but does not give a task of their own. Each has its test assigned to the task that owns the code.

1. **A corrupt or truncated `node.key`** — a half-written key file must fail startup with a clear `LiveError::Config`, never a panic or a silently regenerated identity that changes the node's name. (Task 2)
2. **A malformed entry in `cluster.trusted_keys`** — a string that is not `ed25519:` + 64 hex must be rejected by config validation at startup, not on the first join attempt. (Task 4)
3. **A record whose `node_id` does not match the signing key** — a peer that signs correctly with key A while sending a `NodeRecord` claiming to be node B must be rejected; otherwise the signature proves nothing about the record. (Task 5)
4. **An empty peer table** — no seeds and an empty directory must leave the membership loop idle, never dividing by zero or spinning hot on a zero-length fanout. (Task 6)
5. **A peer whose clock runs ahead** — the timestamp window must reject symmetrically, so a future-dated proof is refused exactly as a stale one is. (Task 3)

---

**This is the second half of one plan.** Tasks 1 through 5 are in
`2026-09-24-distributed-p2p-clustering-phase-1.md` and must be complete first:
they produce `NodeIdentity`, `proof::{sign_request, verify_request}`,
`admission::{Admission, Decision, ticket, build}`, and the `AppState::identity`
/ `AppState::admission` accessors that every task below consumes.

---

### Task 6: The peer table

Closes defects 7, 8, and the O(n²) half of 9.

**Files:**
- Create: `src/cluster/membership.rs`
- Modify: `src/cluster/mod.rs` (`run`), `src/config.rs` (`fanout`)
- Test: in-file `#[cfg(test)] mod tests`

**Interfaces:**
- Produces: `pub struct PeerTable`, `PeerTable::new(seeds: Vec<String>) -> Self`, `PeerTable::seed_from_directory(&mut self, nodes: &[NodeRecord])`, `PeerTable::insert(&mut self, endpoint: String)`, `PeerTable::due(&mut self, now_millis: u64, fanout: usize) -> Vec<String>`, `PeerTable::record_success(&mut self, endpoint: &str, now_millis: u64)`, `PeerTable::record_failure(&mut self, endpoint: &str, now_millis: u64, ceiling_millis: u64)`, `PeerTable::evict(&mut self, endpoint: &str)`, `PeerTable::len(&self) -> usize`

- [ ] **Step 1: Write the failing tests**

Create `src/cluster/membership.rs` with only this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn endpoints(count: usize) -> Vec<String> {
        (0..count).map(|i| format!("https://node{i:02}.example")).collect()
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
        assert_eq!(seen.len(), 20, "every peer must be contacted within n/fanout rounds");
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
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --locked cluster::membership -- --test-threads=1`
Expected: FAIL to compile.

- [ ] **Step 3: Write the implementation**

Above the test module:

```rust
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
        let mut table = Self { peers: BTreeMap::new(), cursor: 0 };
        for seed in seeds {
            table.peers.insert(
                seed,
                PeerState { is_seed: true, failures: 0, next_attempt_millis: 0 },
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
        let ordered: Vec<&String> = self.peers.keys().collect();
        let start = self.cursor % ordered.len();
        let mut due = Vec::new();
        for offset in 0..ordered.len() {
            if due.len() >= fanout {
                break;
            }
            let endpoint = ordered[(start + offset) % ordered.len()];
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
```

- [ ] **Step 4: Run to verify they pass**

Run: `cargo test --locked cluster::membership -- --test-threads=1`
Expected: PASS, 5 tests.

- [ ] **Step 5: Add `cluster.fanout` and wire the table into the loop**

In `src/config.rs` add `pub fanout: usize,` to `ClusterConfig`, default `8`, and add `|| self.cluster.fanout == 0` to the existing bounds check in `validate_cluster`.

In `src/cluster/mod.rs`'s `run`, replace the `BTreeSet<String>` peer set with `PeerTable::new(seeds)`, call `table.seed_from_directory(&state.nodes().list().unwrap_or_default())` once before the loop, drive each round from `table.due(now_millis(), config.fanout)`, and call `record_success`/`record_failure` with `config.node_ttl_secs * 1000` as the ceiling. After pruning, call `table.evict(endpoint)` for every endpoint the prune removed.

- [ ] **Step 6: Run the full suite**

Run: `cargo test --workspace --all-targets --locked -- --test-threads=1`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src/cluster/membership.rs src/cluster/mod.rs src/config.rs
git commit -m "feat(cluster): every peer gets a turn, and a dead one gets a rest"
```

---

### Task 7: Ownership over admitted members, and the membership epoch

Closes defect 5 for outsiders and adds the epoch that detects defect 6.

**Files:**
- Modify: `src/ownership.rs`, `src/app.rs` (`cluster_owner`), `src/modules/control_plane.rs`
- Test: in-file tests in `src/ownership.rs`

**Interfaces:**
- Produces: `ownership::owner_for_operation(resource, nodes, required_operation, admitted: &BTreeSet<String>) -> Option<&NodeRecord>` (fourth parameter added), and `ownership::epoch(admitted: &BTreeSet<String>) -> String`.

- [ ] **Step 1: Write the failing tests**

Add to `src/ownership.rs`'s test module:

```rust
    // Defect 5: an unadmitted identity cannot be chosen, however its id hashes.
    #[test]
    fn only_admitted_members_can_own_state() {
        let nodes = vec![node("alpha"), node("bravo"), node("charlie")];
        let admitted: BTreeSet<String> = ["bravo".to_owned()].into_iter().collect();
        for index in 0..32 {
            let selected = owner_for_operation(&format!("file:{index}"), &nodes, None, &admitted)
                .expect("an admitted owner");
            assert_eq!(selected.node_id, "bravo");
        }
    }

    #[test]
    fn no_admitted_member_means_no_owner() {
        let nodes = vec![node("alpha")];
        assert!(owner_for_operation("file:x", &nodes, None, &BTreeSet::new()).is_none());
    }

    #[test]
    fn the_epoch_tracks_the_admitted_set_and_not_its_order() {
        let forward: BTreeSet<String> = ["a".to_owned(), "b".to_owned()].into_iter().collect();
        let reverse: BTreeSet<String> = ["b".to_owned(), "a".to_owned()].into_iter().collect();
        assert_eq!(epoch(&forward), epoch(&reverse));

        let grown: BTreeSet<String> = ["a".to_owned(), "b".to_owned(), "c".to_owned()]
            .into_iter()
            .collect();
        assert_ne!(epoch(&forward), epoch(&grown));
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --locked ownership -- --test-threads=1`
Expected: FAIL to compile — the arity changed and `epoch` is undefined.

- [ ] **Step 3: Implement**

In `src/ownership.rs`, add `admitted: &BTreeSet<String>` as the fourth parameter of `owner_for_operation` and extend the `filter` closure with `&& admitted.contains(&node.node_id)`. Update `owner` to pass it through. Then add:

```rust
/// A digest of the admitted set. It carries no ordering: a receiver compares it
/// for equality with its own and refuses a mismatch, rather than trying to
/// decide which of two epochs is newer.
pub fn epoch(admitted: &BTreeSet<String>) -> String {
    let mut hasher = blake3::Hasher::new();
    for node_id in admitted {
        hasher.update(node_id.as_bytes());
        hasher.update(&[0]);
    }
    hasher.finalize().to_hex()[..16].to_owned()
}
```

Update `AppState::cluster_owner` to pass `self.admission().admitted()`.

- [ ] **Step 4: Run to verify they pass**

Run: `cargo test --locked ownership -- --test-threads=1`
Expected: PASS.

- [ ] **Step 5: Refuse a mismatched epoch**

In `src/modules/control_plane.rs`, after `authorize_cluster_request` succeeds, if the request carries `x-hologram-cluster-epoch` and it differs from `crate::ownership::epoch(&state.admission().admitted())`, return `LiveError::Conflict` with the local epoch in the message. `LiveError::Conflict` already maps to HTTP 409. Have `contact_peer` send the header.

- [ ] **Step 6: Run the full suite and commit**

Run: `cargo test --workspace --all-targets --locked -- --test-threads=1`
Expected: PASS.

```bash
git add src/ownership.rs src/app.rs src/modules/control_plane.rs src/cluster/mod.rs
git commit -m "feat(cluster): only admitted members own state, and the set is named"
```

---

### Task 8: Replication resilience and cadence

Closes the remaining half of defect 9. The in-memory transfer is left alone on purpose — Phase 2 replaces it with `iroh-blobs`.

**Files:**
- Modify: `src/cluster/replication.rs`, `src/cluster/mod.rs`, `src/config.rs`

- [ ] **Step 1: Write the failing test**

Add to `src/cluster/replication.rs`'s test module:

```rust
    #[test]
    fn an_object_failure_does_not_end_the_round() {
        let mut outcome = RoundOutcome::default();
        outcome.record_object_failure("blake3:aa", &LiveError::Transport("gone".to_owned()));
        outcome.record_object_failure("blake3:bb", &LiveError::Capability("too big".to_owned()));
        outcome.record_object_stored();
        assert_eq!(outcome.failed, 2);
        assert_eq!(outcome.stored, 1);
        assert!(outcome.should_continue(), "per-object failures never end a round");
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --locked cluster::replication -- --test-threads=1`
Expected: FAIL to compile — `RoundOutcome` is undefined.

- [ ] **Step 3: Implement**

Add to `src/cluster/replication.rs`:

```rust
/// Per-object outcomes for one replication round. Object-level failures are
/// counted and logged; only an inventory-level transport failure ends a round.
#[derive(Debug, Default)]
pub(super) struct RoundOutcome {
    pub stored: usize,
    pub failed: usize,
}

impl RoundOutcome {
    pub fn record_object_stored(&mut self) {
        self.stored = self.stored.saturating_add(1);
    }

    pub fn record_object_failure(&mut self, id: &str, error: &LiveError) {
        self.failed = self.failed.saturating_add(1);
        tracing::debug!(object = %id, %error, "skipping a cluster object this round");
    }

    pub const fn should_continue(&self) -> bool {
        true
    }
}
```

In `replicate_peer`, wrap the per-object fetch-and-store body in an inner `async` block returning `Result<()>`; on `Err`, call `record_object_failure` and `continue` instead of propagating with `?`. Keep `?` on the inventory request and decode. Log the outcome once per peer at the end.

- [ ] **Step 4: Add the cadence**

In `src/config.rs` add `pub replication_interval_secs: u64,` to `ClusterConfig`, default `60`, and add `|| self.cluster.replication_interval_secs == 0` to the bounds check. In `src/cluster/mod.rs`, track `last_replication_millis` and call `replicate_peer` only when `now - last >= replication_interval_secs * 1000`, leaving the heartbeat on its own interval.

- [ ] **Step 5: Run and commit**

Run: `cargo test --workspace --all-targets --locked -- --test-threads=1`
Expected: PASS.

```bash
git add src/cluster/replication.rs src/cluster/mod.rs src/config.rs
git commit -m "fix(cluster): one bad object no longer ends the round"
```

---

### Task 9: End-to-end proof and documentation

**Files:**
- Modify: `tests/cluster_e2e.rs`, `docs/superpowers/specs/2026-09-21-server-cluster-membership-design.md`
- Create: `specs/adrs/035-per-node-cluster-identity.md`

- [ ] **Step 1: Write the failing end-to-end tests**

Add to `tests/cluster_e2e.rs`, reusing its existing `start`, `start_without_module`, and `port` helpers:

```rust
// Defect 8: a node restarted with no seeds rejoins from its persisted directory.
#[test]
fn a_restarted_node_rejoins_without_any_configured_seed() {
    hologram_live::util::install_crypto_provider();
    let token = "a sufficiently long shared cluster test token";
    let first = start(port(), None, token);
    let second_port = port();
    let second = start(second_port, Some(first.port), token);

    let client = reqwest::blocking::Client::new();
    await_peer_count(&client, first.port, 2);
    drop(second);

    // Restart on the same state directory with no seeds configured.
    let restarted = restart_without_seeds(second_port, token);
    await_peer_count(&client, restarted.port, 2);
}

// Defect 3: a node holding no admission ticket cannot join.
#[test]
fn a_node_without_the_admission_secret_is_refused() {
    hologram_live::util::install_crypto_provider();
    let first = start(port(), None, "a sufficiently long shared cluster test token");
    let intruder = start(port(), Some(first.port), "an entirely different long secret value");

    let client = reqwest::blocking::Client::new();
    std::thread::sleep(std::time::Duration::from_secs(4));
    let peers: Vec<serde_json::Value> = client
        .get(format!("http://127.0.0.1:{}/api/v1/nodes", first.port))
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(peers.len(), 1, "the intruder must not appear in the directory");
    drop(intruder);
}
```

Add the two helpers `await_peer_count(client, port, expected)` — polling `/api/v1/nodes` until the count matches or a 15-second deadline expires — and `restart_without_seeds(port, token)`, a copy of `start` that reuses an existing root directory and sets `config.cluster.seeds = Vec::new()`. Give `start_without_module` an extra `root: Option<tempfile::TempDir>` parameter so the state directory can outlive one process.

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --locked --test cluster_e2e -- --test-threads=1`
Expected: both new tests FAIL before Tasks 5–6 are in place; after them, PASS.

- [ ] **Step 3: Run the whole gate**

Run: `just verify`
Expected: every stage passes except `file-size`, which fails on four **pre-existing** violations under `apps/model-hub/web` that this branch does not touch. Confirm with `git stash && ./scripts/check-file-size.sh` that the same four fail on `main`; if any file from this branch appears, split it before continuing.

- [ ] **Step 4: Update the superseded design document**

At the top of `docs/superpowers/specs/2026-09-21-server-cluster-membership-design.md`, add:

```markdown
> **Superseded** by `2026-09-24-distributed-p2p-clustering-design.md`. The
> shared-secret proof described below has been replaced by per-node ed25519
> identity and admission. This document is retained for history.
```

- [ ] **Step 5: Write the ADR**

Create `specs/adrs/035-per-node-cluster-identity.md` following the house format of the existing ADRs: context (the nine audited defects, naming the identical-`server_id` collision as the blocking one), decision (ed25519 identity, canonical request proof, `Admission` trait with allowlist and token trust-on-first-use, ownership restricted to admitted members), consequences (one new BSD-3-Clause dependency recorded in `DEPENDENCIES.md`; `CapabilityManifest.server_id` changes value; the secret bytes become the Phase 2 iroh `SecretKey`), and the rejected alternatives (Veilid on licence and the existing gate, Reticulum on maturity and fit).

- [ ] **Step 6: Commit**

```bash
git add tests/cluster_e2e.rs docs/superpowers/specs/ specs/adrs/035-per-node-cluster-identity.md
git commit -m "test(cluster): two daemons converge, and a stranger does not"
```

---

## Done when

- `cargo test --workspace --all-targets --locked -- --test-threads=1` passes.
- `cargo clippy --workspace --all-targets --locked -- -D warnings` is silent.
- `just verify` passes every stage but the four pre-existing `apps/model-hub/web` file-size violations.
- Two daemons started from byte-identical configuration on different hosts hold distinct identities and both appear in each other's `/api/v1/nodes`.
- A daemon restarted with no configured seeds rejoins from its persisted directory.
- A daemon holding the wrong admission secret never enters the directory.
