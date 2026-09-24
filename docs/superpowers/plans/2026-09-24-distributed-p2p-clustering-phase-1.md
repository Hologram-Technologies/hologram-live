# Distributed P2P Clustering — Phase 1 Implementation Plan

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

### Task 1: Split `src/cluster.rs` into a module directory

A pure move with no behavior change, done first so every later task edits a file that already exists in its final home.

**Files:**
- Create: `src/cluster/mod.rs`, `src/cluster/replication.rs`
- Delete: `src/cluster.rs`
- Test: existing `tests/cluster_e2e.rs` and the in-file unit tests, unchanged

**Interfaces:**
- Consumes: nothing
- Produces: `crate::cluster::{JOIN_PATH, OBJECTS_PATH, OBJECT_PATH, TIMESTAMP_HEADER, SIGNATURE_HEADER, MAX_JOIN_BYTES, TOKEN_FILE, load_or_create_token, spawn, verify, validate_node_record}` — the same public surface at the same paths, so no caller changes.

- [ ] **Step 1: Record the current test baseline**

Run: `cargo test --workspace --all-targets --locked -- --test-threads=1 cluster`
Expected: PASS. Note the number of passing tests; Step 4 must match it.

- [ ] **Step 2: Move the file**

```bash
mkdir -p src/cluster
git mv src/cluster.rs src/cluster/mod.rs
```

- [ ] **Step 3: Move replication into its own file**

Cut `replicate_peer`, `cluster_url`, and `signed_get` out of `src/cluster/mod.rs` into a new `src/cluster/replication.rs`. Add to the top of the new file:

```rust
//! Immutable object reconciliation against a cluster peer.

use crate::app::AppState;
use crate::error::{LiveError, Result};
use crate::protocol::{ObjectPage, ObjectQuery};
use super::{sign, OBJECTS_PATH, SIGNATURE_HEADER, TIMESTAMP_HEADER};
use crate::util::now_millis;
```

Make the three moved functions `pub(super)`, and move the `peer_object_paths_do_not_replace_the_advertised_origin` test into `replication.rs`'s own `#[cfg(test)] mod tests`. In `src/cluster/mod.rs` add `mod replication;` and `use replication::replicate_peer;`.

- [ ] **Step 4: Verify nothing changed**

Run: `cargo test --workspace --all-targets --locked -- --test-threads=1 cluster`
Expected: PASS, with the same test count as Step 1.

Run: `cargo clippy --workspace --all-targets --locked -- -D warnings`
Expected: no warnings.

- [ ] **Step 5: Commit**

```bash
git add src/cluster/
git commit -m "refactor(cluster): the membership module gets room to grow"
```

---

### Task 2: Per-node ed25519 identity

**Files:**
- Create: `src/cluster/identity.rs`
- Modify: `Cargo.toml` (add `ed25519-dalek`), `DEPENDENCIES.md`
- Test: in-file `#[cfg(test)] mod tests` in `src/cluster/identity.rs`

**Interfaces:**
- Consumes: `crate::error::{LiveError, Result}`, `crate::util::hex`
- Produces:
  - `pub const KEY_FILE: &str = "node.key";`
  - `pub struct NodeIdentity`
  - `NodeIdentity::load_or_create(path: &Path) -> Result<NodeIdentity>`
  - `NodeIdentity::node_id(&self) -> String` — `"ed25519:"` + 64 lowercase hex
  - `NodeIdentity::sign(&self, message: &[u8]) -> String` — 128 lowercase hex
  - `NodeIdentity::secret_bytes(&self) -> [u8; 32]` — Phase 2 builds the iroh `SecretKey` from this
  - `pub fn verify_signature(node_id: &str, message: &[u8], signature: &str) -> Result<()>`
  - `pub fn parse_node_id(node_id: &str) -> Result<VerifyingKey>`
  - `pub fn unhex(text: &str) -> Option<Vec<u8>>`

- [ ] **Step 1: Add the dependency**

In `Cargo.toml`, directly after the `clap` line in `[dependencies]`:

```toml
# Per-node cluster identity. The public key is the node_id, and in Phase 2 the
# same 32 secret bytes construct the iroh SecretKey, so identity is continuous
# across the transport change. BSD-3-Clause; already in Cargo.lock as a
# transitive, so this does not move the lock.
ed25519-dalek = { version = "3", default-features = false, features = ["std"] }
```

Run: `cargo check --workspace --locked`
Expected: compiles, and `git diff --stat Cargo.lock` shows no change to the `ed25519-dalek` version.

- [ ] **Step 2: Write the failing tests**

Create `src/cluster/identity.rs` containing only the test module for now:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_round_trips_and_is_private() {
        let directory = tempfile::tempdir().expect("temporary state directory");
        let path = directory.path().join(KEY_FILE);

        let first = NodeIdentity::load_or_create(&path).expect("create identity");
        let second = NodeIdentity::load_or_create(&path).expect("reload identity");

        assert_eq!(first.node_id(), second.node_id());
        assert!(first.node_id().starts_with("ed25519:"));
        assert_eq!(first.node_id().len(), "ed25519:".len() + 64);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path)
                .expect("key metadata")
                .permissions()
                .mode();
            assert_eq!(mode & 0o077, 0);
        }
    }

    // Defect 1: two nodes with byte-identical configuration must still be
    // distinct members. The old derived server_id made them the same node.
    #[test]
    fn two_nodes_with_identical_configuration_have_distinct_identities() {
        let first_dir = tempfile::tempdir().expect("first state directory");
        let second_dir = tempfile::tempdir().expect("second state directory");
        let first = NodeIdentity::load_or_create(&first_dir.path().join(KEY_FILE))
            .expect("first identity");
        let second = NodeIdentity::load_or_create(&second_dir.path().join(KEY_FILE))
            .expect("second identity");
        assert_ne!(first.node_id(), second.node_id());
    }

    #[test]
    fn a_signature_verifies_only_for_its_own_message_and_signer() {
        let directory = tempfile::tempdir().expect("temporary state directory");
        let identity =
            NodeIdentity::load_or_create(&directory.path().join(KEY_FILE)).expect("identity");
        let signature = identity.sign(b"cluster message");

        verify_signature(&identity.node_id(), b"cluster message", &signature)
            .expect("valid signature");
        assert!(verify_signature(&identity.node_id(), b"other message", &signature).is_err());

        let other_dir = tempfile::tempdir().expect("other state directory");
        let other =
            NodeIdentity::load_or_create(&other_dir.path().join(KEY_FILE)).expect("other identity");
        assert!(verify_signature(&other.node_id(), b"cluster message", &signature).is_err());
    }

    // Review Focus 1: a truncated key must fail loudly, never regenerate.
    #[test]
    fn a_corrupt_key_file_is_a_configuration_error() {
        let directory = tempfile::tempdir().expect("temporary state directory");
        let path = directory.path().join(KEY_FILE);
        std::fs::write(&path, "not a key\n").expect("write corrupt key");

        let error = NodeIdentity::load_or_create(&path).expect_err("corrupt key must fail");
        assert!(matches!(error, LiveError::Config(_)), "got {error:?}");
        assert!(
            !error.to_string().contains("not a key"),
            "the error must not echo key file contents"
        );
    }

    #[test]
    fn a_malformed_node_id_is_rejected() {
        assert!(parse_node_id("ed25519:zz").is_err());
        assert!(parse_node_id(&format!("ed25519:{}", "a".repeat(64))).is_err());
        assert!(parse_node_id("blake3:0123").is_err());
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test --locked cluster::identity -- --test-threads=1`
Expected: FAIL to compile — `NodeIdentity`, `KEY_FILE`, `verify_signature`, `parse_node_id` are not defined.

- [ ] **Step 4: Write the implementation**

Above the test module in `src/cluster/identity.rs`:

```rust
//! Per-node cluster identity.
//!
//! A node's name is its ed25519 public key, so membership cannot be spoofed by
//! claiming someone else's identifier and two identically configured hosts are
//! still distinct members. Phase 2 builds the iroh `SecretKey` from the same
//! secret bytes, keeping one identity across the transport change.

use crate::error::{LiveError, Result};
use crate::util::hex;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

pub const KEY_FILE: &str = "node.key";
const NODE_ID_PREFIX: &str = "ed25519:";

pub struct NodeIdentity {
    signing: SigningKey,
}

impl NodeIdentity {
    pub fn load_or_create(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                secure_key_file(path)?;
                let bytes = unhex(text.trim()).ok_or_else(|| {
                    LiveError::Config(format!(
                        "{} is not a valid node key; remove it to generate a new identity",
                        path.display()
                    ))
                })?;
                let bytes: [u8; 32] = bytes.as_slice().try_into().map_err(|_| {
                    LiveError::Config(format!(
                        "{} is not a valid node key; remove it to generate a new identity",
                        path.display()
                    ))
                })?;
                Ok(Self {
                    signing: SigningKey::from_bytes(&bytes),
                })
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => create_key_file(path),
            Err(error) => Err(LiveError::io(path, error)),
        }
    }

    pub fn node_id(&self) -> String {
        format!(
            "{NODE_ID_PREFIX}{}",
            hex(self.signing.verifying_key().as_bytes())
        )
    }

    pub fn sign(&self, message: &[u8]) -> String {
        hex(&self.signing.sign(message).to_bytes())
    }

    /// The raw secret. Phase 2 constructs the iroh `SecretKey` from these bytes
    /// so a peer dials exactly the identity it already admitted.
    pub fn secret_bytes(&self) -> [u8; 32] {
        self.signing.to_bytes()
    }
}

fn create_key_file(path: &Path) -> Result<NodeIdentity> {
    let mut secret = [0_u8; 32];
    getrandom::fill(&mut secret)
        .map_err(|error| LiveError::Io(format!("generate node key: {error}")))?;

    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(path) {
        Ok(mut file) => {
            let encoded = hex(&secret);
            file.write_all(encoded.as_bytes())
                .and_then(|()| file.write_all(b"\n"))
                .and_then(|()| file.sync_all())
                .map_err(|error| LiveError::io(path, error))?;
            Ok(NodeIdentity {
                signing: SigningKey::from_bytes(&secret),
            })
        }
        // Another process won the race; read what it wrote.
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            NodeIdentity::load_or_create(path)
        }
        Err(error) => Err(LiveError::io(path, error)),
    }
}

#[cfg(unix)]
fn secure_key_file(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let metadata = std::fs::metadata(path).map_err(|error| LiveError::io(path, error))?;
    if metadata.permissions().mode() & 0o077 != 0 {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(|error| LiveError::io(path, error))?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn secure_key_file(_path: &Path) -> Result<()> {
    Ok(())
}

pub fn parse_node_id(node_id: &str) -> Result<VerifyingKey> {
    let encoded = node_id.strip_prefix(NODE_ID_PREFIX).ok_or_else(|| {
        LiveError::Protocol(format!("cluster node id must start with {NODE_ID_PREFIX}"))
    })?;
    let bytes = unhex(encoded)
        .ok_or_else(|| LiveError::Protocol("cluster node id is not hexadecimal".to_owned()))?;
    let bytes: [u8; 32] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| LiveError::Protocol("cluster node id must be 32 bytes".to_owned()))?;
    VerifyingKey::from_bytes(&bytes)
        .map_err(|_| LiveError::Protocol("cluster node id is not a valid public key".to_owned()))
}

pub fn verify_signature(node_id: &str, message: &[u8], signature: &str) -> Result<()> {
    let key = parse_node_id(node_id)?;
    let bytes = unhex(signature).ok_or_else(|| {
        LiveError::Authentication("cluster signature is not hexadecimal".to_owned())
    })?;
    let bytes: [u8; 64] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| LiveError::Authentication("cluster signature must be 64 bytes".to_owned()))?;
    key.verify(message, &Signature::from_bytes(&bytes))
        .map_err(|_| LiveError::Authentication("invalid cluster signature".to_owned()))
}

pub fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    let mut bytes = Vec::with_capacity(text.len() / 2);
    for pair in text.as_bytes().chunks(2) {
        let high = char::from(pair[0]).to_digit(16)?;
        let low = char::from(pair[1]).to_digit(16)?;
        bytes.push(((high << 4) | low) as u8);
    }
    Some(bytes)
}
```

Add `pub(crate) mod identity;` to `src/cluster/mod.rs`. It must be `pub(crate)`, not private: Task 4 calls `parse_node_id` from `src/config.rs` and Task 5 calls `NodeIdentity` from `src/app.rs`.

Note on the `a_malformed_node_id_is_rejected` test: `"a".repeat(64)` decodes to 32 bytes of `0xaa`, which is not a valid compressed Edwards point, so `VerifyingKey::from_bytes` rejects it. That is the case the test pins.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --locked cluster::identity -- --test-threads=1`
Expected: PASS, 5 tests.

- [ ] **Step 6: Record the dependency**

In `DEPENDENCIES.md`, add to the table after the `blake3` row:

```markdown
| `ed25519-dalek`                              | per-node cluster identity: the public key is the node id and signs every cluster request |
```

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock DEPENDENCIES.md src/cluster/identity.rs src/cluster/mod.rs
git commit -m "feat(cluster): a node's name is its own public key"
```

---

### Task 3: The canonical request proof

Closes defect 4. Today `signed_get` signs `(timestamp, empty body)`, so a captured header pair replays against any route on any peer.

**Files:**
- Create: `src/cluster/proof.rs`
- Test: in-file `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: `crate::cluster::identity::{verify_signature, NodeIdentity}`
- Produces:
  - `pub const NODE_HEADER: &str = "x-hologram-cluster-node";`
  - `pub const TIMESTAMP_HEADER: &str = "x-hologram-cluster-timestamp";`
  - `pub const SIGNATURE_HEADER: &str = "x-hologram-cluster-signature";`
  - `pub const TICKET_HEADER: &str = "x-hologram-cluster-ticket";`
  - `pub struct RequestProof { pub node_id: String, pub timestamp: String, pub signature: String }`
  - `pub fn canonical_query(raw: Option<&str>) -> String`
  - `pub fn preimage(method: &str, path: &str, query: Option<&str>, recipient: &str, timestamp: &str, body: &[u8]) -> Vec<u8>`
  - `pub fn sign_request(identity: &NodeIdentity, recipient: &str, method: &str, path: &str, query: Option<&str>, body: &[u8]) -> RequestProof`
  - `pub fn verify_request(proof: &RequestProof, recipient: &str, method: &str, path: &str, query: Option<&str>, body: &[u8]) -> Result<()>`

- [ ] **Step 1: Write the failing tests**

Create `src/cluster/proof.rs` with only this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::cluster::identity::{NodeIdentity, KEY_FILE};

    fn identity() -> (tempfile::TempDir, NodeIdentity) {
        let directory = tempfile::tempdir().expect("temporary state directory");
        let identity =
            NodeIdentity::load_or_create(&directory.path().join(KEY_FILE)).expect("identity");
        (directory, identity)
    }

    #[test]
    fn the_preimage_is_stable_and_field_separated() {
        let bytes = preimage(
            "GET",
            "/api/v1/cluster/objects",
            Some("limit=10"),
            "ed25519:ff",
            "1700000000000",
            b"",
        );
        let text = String::from_utf8(bytes).expect("preimage is utf-8");
        assert_eq!(
            text,
            concat!(
                "dev.hologram.live.cluster.v2\n",
                "GET\n",
                "/api/v1/cluster/objects\n",
                "limit=10\n",
                "ed25519:ff\n",
                "1700000000000\n",
                "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
            )
        );
    }

    #[test]
    fn query_parameter_order_does_not_change_the_proof() {
        assert_eq!(
            canonical_query(Some("limit=10&cursor=abc")),
            canonical_query(Some("cursor=abc&limit=10"))
        );
        assert_eq!(canonical_query(None), "");
        assert_eq!(canonical_query(Some("")), "");
    }

    #[test]
    fn a_proof_verifies_only_for_the_exact_request_it_signed() {
        let (_dir, identity) = identity();
        let recipient = "ed25519:aa";
        let proof = sign_request(
            &identity,
            recipient,
            "GET",
            "/api/v1/cluster/objects",
            Some("limit=10"),
            b"",
        );

        verify_request(&proof, recipient, "GET", "/api/v1/cluster/objects", Some("limit=10"), b"")
            .expect("the proof it signed");

        // Every bound field, altered one at a time.
        assert!(verify_request(&proof, recipient, "POST", "/api/v1/cluster/objects", Some("limit=10"), b"").is_err(), "method");
        assert!(verify_request(&proof, recipient, "GET", "/api/v1/cluster/join", Some("limit=10"), b"").is_err(), "path");
        assert!(verify_request(&proof, recipient, "GET", "/api/v1/cluster/objects", Some("limit=99"), b"").is_err(), "query");
        assert!(verify_request(&proof, "ed25519:bb", "GET", "/api/v1/cluster/objects", Some("limit=10"), b"").is_err(), "recipient");
        assert!(verify_request(&proof, recipient, "GET", "/api/v1/cluster/objects", Some("limit=10"), b"body").is_err(), "body");
    }

    // Review Focus 5: the window must reject a future proof as firmly as a stale one.
    #[test]
    fn the_clock_window_rejects_both_directions() {
        let (_dir, identity) = identity();
        let recipient = "ed25519:aa";

        for offset in [
            crate::util::now_millis().saturating_sub(MAX_CLOCK_SKEW_MILLIS + 1_000),
            crate::util::now_millis().saturating_add(MAX_CLOCK_SKEW_MILLIS + 1_000),
        ] {
            let timestamp = offset.to_string();
            let signature = identity.sign(&preimage("GET", "/p", None, recipient, &timestamp, b""));
            let proof = RequestProof {
                node_id: identity.node_id(),
                timestamp,
                signature,
            };
            assert!(
                verify_request(&proof, recipient, "GET", "/p", None, b"").is_err(),
                "offset {offset} must be outside the window"
            );
        }
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --locked cluster::proof -- --test-threads=1`
Expected: FAIL to compile — nothing in `super` is defined.

- [ ] **Step 3: Write the implementation**

Above the test module in `src/cluster/proof.rs`:

```rust
//! The authenticated proof carried by every cluster request.
//!
//! The signature binds the method, path, query, recipient, timestamp, and body
//! digest, so a captured proof cannot be replayed against a different route or
//! a different peer.

use crate::cluster::identity::{verify_signature, NodeIdentity};
use crate::error::{LiveError, Result};
use crate::util::now_millis;

pub const NODE_HEADER: &str = "x-hologram-cluster-node";
pub const TIMESTAMP_HEADER: &str = "x-hologram-cluster-timestamp";
pub const SIGNATURE_HEADER: &str = "x-hologram-cluster-signature";
pub const TICKET_HEADER: &str = "x-hologram-cluster-ticket";
const SIGNING_CONTEXT: &str = "dev.hologram.live.cluster.v2";
const MAX_CLOCK_SKEW_MILLIS: u64 = 30_000;

#[derive(Debug, Clone)]
pub struct RequestProof {
    pub node_id: String,
    pub timestamp: String,
    pub signature: String,
}

/// Query parameters sorted as raw `key=value` segments and rejoined.
///
/// Sorting the already-encoded segments avoids any decode/re-encode asymmetry
/// between the signer and the verifier.
pub fn canonical_query(raw: Option<&str>) -> String {
    let Some(raw) = raw else {
        return String::new();
    };
    let mut segments: Vec<&str> = raw.split('&').filter(|s| !s.is_empty()).collect();
    segments.sort_unstable();
    segments.join("&")
}

pub fn preimage(
    method: &str,
    path: &str,
    query: Option<&str>,
    recipient: &str,
    timestamp: &str,
    body: &[u8],
) -> Vec<u8> {
    let digest = blake3::hash(body).to_hex();
    format!(
        "{SIGNING_CONTEXT}\n{method}\n{path}\n{}\n{recipient}\n{timestamp}\n{digest}",
        canonical_query(query)
    )
    .into_bytes()
}

pub fn sign_request(
    identity: &NodeIdentity,
    recipient: &str,
    method: &str,
    path: &str,
    query: Option<&str>,
    body: &[u8],
) -> RequestProof {
    let timestamp = now_millis().to_string();
    let signature = identity.sign(&preimage(method, path, query, recipient, &timestamp, body));
    RequestProof {
        node_id: identity.node_id(),
        timestamp,
        signature,
    }
}

pub fn verify_request(
    proof: &RequestProof,
    recipient: &str,
    method: &str,
    path: &str,
    query: Option<&str>,
    body: &[u8],
) -> Result<()> {
    let timestamp_millis = proof
        .timestamp
        .parse::<u64>()
        .map_err(|_| LiveError::Authentication("invalid cluster timestamp".to_owned()))?;
    // abs_diff makes the window symmetric: a clock running ahead is refused
    // exactly as one running behind.
    if now_millis().abs_diff(timestamp_millis) > MAX_CLOCK_SKEW_MILLIS {
        return Err(LiveError::Authentication(
            "cluster timestamp is outside the allowed clock window".to_owned(),
        ));
    }
    verify_signature(
        &proof.node_id,
        &preimage(method, path, query, recipient, &proof.timestamp, body),
        &proof.signature,
    )
}
```

Add `pub(crate) mod proof;` to `src/cluster/mod.rs` — Task 5 calls it from `src/modules/control_plane.rs`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --locked cluster::proof -- --test-threads=1`
Expected: PASS, 4 tests. If `the_preimage_is_stable_and_field_separated` fails on the digest, confirm `blake3::hash(b"")` is `af1349b9…3262`.

- [ ] **Step 5: Commit**

```bash
git add src/cluster/proof.rs src/cluster/mod.rs
git commit -m "feat(cluster): a proof that names the route and the recipient"
```

---

### Task 4: Admission

Closes defect 3. The shared token stops being the whole trust model and becomes an admission ticket.

**Files:**
- Create: `src/cluster/admission.rs`
- Modify: `src/config.rs` (add `trusted_keys`, `admission`; extend `validate_cluster`)
- Test: in-file `#[cfg(test)] mod tests` in both files

**Interfaces:**
- Consumes: `crate::cluster::identity::parse_node_id`
- Produces:
  - `pub enum Decision { Admit, Deny(String) }`
  - `pub trait Admission: Send + Sync { fn authorize(&self, node_id: &str, ticket: Option<&str>) -> Decision; fn admitted(&self) -> BTreeSet<String>; }`
  - `pub struct AllowlistAdmission; AllowlistAdmission::new(trusted: Vec<String>) -> Self`
  - `pub struct TokenAdmission; TokenAdmission::new(token: String, pinned_path: PathBuf, trusted: Vec<String>) -> Result<Self>`
  - `pub fn ticket(token: &str, node_id: &str) -> String`
  - `pub fn build(config: &crate::config::ClusterConfig, token: Option<&str>, state_dir: &Path) -> Result<Arc<dyn Admission>>`

- [ ] **Step 1: Write the failing tests**

Create `src/cluster/admission.rs` with only this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const ALICE: &str = "ed25519:d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";
    const BOB: &str = "ed25519:3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c";

    #[test]
    fn the_allowlist_admits_only_listed_identities() {
        let admission = AllowlistAdmission::new(vec![ALICE.to_owned()]);
        assert!(matches!(admission.authorize(ALICE, None), Decision::Admit));
        assert!(matches!(admission.authorize(BOB, None), Decision::Deny(_)));
    }

    #[test]
    fn a_valid_ticket_pins_a_new_identity_and_survives_reload() {
        let directory = tempfile::tempdir().expect("temporary state directory");
        let path = directory.path().join("cluster-pinned.json");
        let token = "a sufficiently long shared cluster admission token";

        let admission =
            TokenAdmission::new(token.to_owned(), path.clone(), Vec::new()).expect("admission");
        // Without a ticket an unknown identity is refused.
        assert!(matches!(admission.authorize(ALICE, None), Decision::Deny(_)));
        // With the right ticket it is admitted, and pinned.
        assert!(matches!(
            admission.authorize(ALICE, Some(&ticket(token, ALICE))),
            Decision::Admit
        ));
        // Pinned identities no longer need the ticket.
        assert!(matches!(admission.authorize(ALICE, None), Decision::Admit));

        let reloaded = TokenAdmission::new(token.to_owned(), path, Vec::new()).expect("reload");
        assert!(matches!(reloaded.authorize(ALICE, None), Decision::Admit));
    }

    #[test]
    fn a_ticket_for_one_identity_does_not_admit_another() {
        let directory = tempfile::tempdir().expect("temporary state directory");
        let token = "a sufficiently long shared cluster admission token";
        let admission = TokenAdmission::new(
            token.to_owned(),
            directory.path().join("cluster-pinned.json"),
            Vec::new(),
        )
        .expect("admission");

        // Alice's ticket presented by Bob.
        assert!(matches!(
            admission.authorize(BOB, Some(&ticket(token, ALICE))),
            Decision::Deny(_)
        ));
        assert!(matches!(
            admission.authorize(ALICE, Some("not a ticket")),
            Decision::Deny(_)
        ));
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --locked cluster::admission -- --test-threads=1`
Expected: FAIL to compile.

- [ ] **Step 3: Write the implementation**

Above the test module in `src/cluster/admission.rs`:

```rust
//! Who may participate in this cluster.
//!
//! This is the seam between a closed cluster and an open network. A future
//! capability-grant implementation satisfies the same trait without touching
//! transport or membership.

use crate::cluster::identity::parse_node_id;
use crate::error::{LiveError, Result};
use crate::util::{atomic_write, constant_time_eq, hex};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const TICKET_CONTEXT: &str = "dev.hologram.live.cluster-ticket.v1";
pub const PINNED_FILE: &str = "cluster-pinned.json";

#[derive(Debug)]
pub enum Decision {
    Admit,
    Deny(String),
}

pub trait Admission: Send + Sync {
    fn authorize(&self, node_id: &str, ticket: Option<&str>) -> Decision;
    /// The identities currently eligible to own resources.
    fn admitted(&self) -> BTreeSet<String>;
}

/// A ticket proves the bearer holds the cluster token *for this identity*.
pub fn ticket(token: &str, node_id: &str) -> String {
    let key = blake3::derive_key(TICKET_CONTEXT, token.as_bytes());
    let mut hasher = blake3::Hasher::new_keyed(&key);
    hasher.update(node_id.as_bytes());
    hex(hasher.finalize().as_bytes())
}

pub struct AllowlistAdmission {
    trusted: BTreeSet<String>,
}

impl AllowlistAdmission {
    pub fn new(trusted: Vec<String>) -> Self {
        Self {
            trusted: trusted.into_iter().collect(),
        }
    }
}

impl Admission for AllowlistAdmission {
    fn authorize(&self, node_id: &str, _ticket: Option<&str>) -> Decision {
        if self.trusted.contains(node_id) {
            Decision::Admit
        } else {
            Decision::Deny("node is not in cluster.trusted_keys".to_owned())
        }
    }

    fn admitted(&self) -> BTreeSet<String> {
        self.trusted.clone()
    }
}

pub struct TokenAdmission {
    token: String,
    path: PathBuf,
    pinned: Mutex<BTreeSet<String>>,
}

impl TokenAdmission {
    pub fn new(token: String, path: PathBuf, trusted: Vec<String>) -> Result<Self> {
        let mut pinned: BTreeSet<String> = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => BTreeSet::new(),
            Err(error) => return Err(LiveError::io(&path, error)),
        };
        pinned.extend(trusted);
        Ok(Self {
            token,
            path,
            pinned: Mutex::new(pinned),
        })
    }
}

impl Admission for TokenAdmission {
    fn authorize(&self, node_id: &str, presented: Option<&str>) -> Decision {
        let Ok(mut pinned) = self.pinned.lock() else {
            return Decision::Deny("admission state is poisoned".to_owned());
        };
        if pinned.contains(node_id) {
            return Decision::Admit;
        }
        if parse_node_id(node_id).is_err() {
            return Decision::Deny("node id is not a valid public key".to_owned());
        }
        let Some(presented) = presented else {
            return Decision::Deny("unknown node presented no admission ticket".to_owned());
        };
        let expected = ticket(&self.token, node_id);
        if !constant_time_eq(expected.as_bytes(), presented.as_bytes()) {
            return Decision::Deny("admission ticket is invalid for this node".to_owned());
        }
        pinned.insert(node_id.to_owned());
        // A failed write must not admit silently on the next restart; log and
        // keep the in-memory pin so this round still works.
        match serde_json::to_vec_pretty(&*pinned)
            .map_err(LiveError::from)
            .and_then(|bytes| atomic_write(&self.path, &bytes))
        {
            Ok(()) => {}
            Err(error) => tracing::warn!(%error, "failed to persist pinned cluster identity"),
        }
        Decision::Admit
    }

    fn admitted(&self) -> BTreeSet<String> {
        self.pinned
            .lock()
            .map(|pinned| pinned.clone())
            .unwrap_or_default()
    }
}

pub fn build(
    config: &crate::config::ClusterConfig,
    token: Option<&str>,
    state_dir: &Path,
) -> Result<Arc<dyn Admission>> {
    match config.admission.as_str() {
        "allowlist" => Ok(Arc::new(AllowlistAdmission::new(
            config.trusted_keys.clone(),
        ))),
        "token" => {
            let token = token
                .ok_or_else(|| {
                    LiveError::Config(
                        "cluster.admission \"token\" requires a cluster token".to_owned(),
                    )
                })?
                .to_owned();
            Ok(Arc::new(TokenAdmission::new(
                token,
                state_dir.join(PINNED_FILE),
                config.trusted_keys.clone(),
            )?))
        }
        other => Err(LiveError::Config(format!(
            "unsupported cluster.admission {other:?}; expected token or allowlist"
        ))),
    }
}
```

Add `pub(crate) mod admission;` to `src/cluster/mod.rs` — Task 5 calls `build` from `src/app.rs`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --locked cluster::admission -- --test-threads=1`
Expected: PASS, 3 tests.

- [ ] **Step 5: Add the configuration fields**

In `src/config.rs`, add to `ClusterConfig` after `max_peers`:

```rust
    /// Identities always eligible to participate, as `ed25519:<64 hex>`.
    pub trusted_keys: Vec<String>,
    /// `token` pins a new identity on first contact with a valid ticket;
    /// `allowlist` admits only `trusted_keys`.
    pub admission: String,
```

And to `impl Default for ClusterConfig`:

```rust
            trusted_keys: Vec::new(),
            admission: "token".to_owned(),
```

- [ ] **Step 6: Write the failing config-validation test**

Add to `src/config.rs`'s existing `#[cfg(test)] mod tests`:

```rust
    // Review Focus 2: a bad key must fail at startup, not at the first join.
    #[test]
    fn a_malformed_trusted_key_is_rejected_at_startup() {
        let mut config = AppConfig::default();
        config.cluster.trusted_keys = vec!["not-a-key".to_owned()];
        let error = config.validate().expect_err("malformed trusted key");
        assert!(matches!(error, LiveError::Config(_)), "got {error:?}");
    }

    #[test]
    fn an_unsupported_admission_mode_is_rejected() {
        let mut config = AppConfig::default();
        config.cluster.admission = "anyone".to_owned();
        assert!(config.validate().is_err());
    }
```

- [ ] **Step 7: Run to verify it fails**

Run: `cargo test --locked config::tests::a_malformed_trusted_key -- --test-threads=1`
Expected: FAIL — `validate` currently accepts it.

- [ ] **Step 8: Extend validation**

In `validate_cluster`, immediately before the final `Ok(())`:

```rust
        if !matches!(self.cluster.admission.as_str(), "token" | "allowlist") {
            return Err(LiveError::Config(format!(
                "unsupported cluster.admission {:?}; expected token or allowlist",
                self.cluster.admission
            )));
        }
        for key in &self.cluster.trusted_keys {
            crate::cluster::identity::parse_node_id(key).map_err(|error| {
                LiveError::Config(format!("cluster.trusted_keys entry {key:?}: {error}"))
            })?;
        }
        if self.cluster.admission == "allowlist" && self.cluster.trusted_keys.is_empty() {
            return Err(LiveError::Config(
                "cluster.admission \"allowlist\" requires at least one cluster.trusted_keys entry"
                    .to_owned(),
            ));
        }
```

`identity` is already `pub(crate)` from Task 2, so `parse_node_id` is reachable here.

- [ ] **Step 9: Run to verify it passes**

Run: `cargo test --locked config -- --test-threads=1`
Expected: PASS, including both new tests and every pre-existing config test.

- [ ] **Step 10: Commit**

```bash
git add src/cluster/admission.rs src/cluster/mod.rs src/config.rs
git commit -m "feat(cluster): the shared token admits a node instead of being it"
```

---

### Task 5: Handlers and client use per-node proofs

Replaces the shared-secret HMAC on both sides. Closes defects 1 and 3 end to end.

**Files:**
- Modify: `src/app.rs` (hold `NodeIdentity` + `Arc<dyn Admission>`, drop the derived `server_id`), `src/modules/control_plane.rs` (`verify_cluster_request`, `join_cluster`), `src/cluster/mod.rs` (`contact_peer`), `src/cluster/replication.rs` (`signed_get`)
- Test: in-file tests in `src/modules/control_plane.rs`

**Interfaces:**
- Consumes: `proof::{sign_request, verify_request, RequestProof, NODE_HEADER, TIMESTAMP_HEADER, SIGNATURE_HEADER, TICKET_HEADER}`, `admission::{Admission, Decision, ticket}`, `NodeIdentity`
- Produces: `AppState::identity(&self) -> &NodeIdentity`, `AppState::admission(&self) -> &Arc<dyn Admission>`, and `cluster::authorize_request(state, headers, method, path, query, body) -> Result<String>` returning the verified caller's `node_id`.

- [ ] **Step 1: Write the failing tests**

Add to `src/modules/control_plane.rs`'s test module (create one if absent):

```rust
#[cfg(test)]
mod tests {
    use crate::cluster::identity::{NodeIdentity, KEY_FILE};
    use crate::cluster::proof::sign_request;
    use crate::cluster::proof::verify_request;

    // Defect 4: a proof minted for one peer must not open another.
    #[test]
    fn a_proof_for_one_peer_is_refused_by_another() {
        let dir = tempfile::tempdir().expect("state directory");
        let caller = NodeIdentity::load_or_create(&dir.path().join(KEY_FILE)).expect("identity");
        let proof = sign_request(&caller, "ed25519:aa", "GET", "/api/v1/cluster/objects", None, b"");
        assert!(
            verify_request(&proof, "ed25519:bb", "GET", "/api/v1/cluster/objects", None, b"")
                .is_err()
        );
    }

    // Review Focus 3: signing correctly as A while claiming to be B proves nothing.
    #[test]
    fn a_record_that_disagrees_with_the_signer_is_refused() {
        let dir = tempfile::tempdir().expect("state directory");
        let signer = NodeIdentity::load_or_create(&dir.path().join(KEY_FILE)).expect("identity");
        let other = tempfile::tempdir().expect("other state directory");
        let claimed = NodeIdentity::load_or_create(&other.path().join(KEY_FILE)).expect("other");

        let mut record = crate::protocol::NodeRecord {
            node_id: claimed.node_id(),
            version: "test".to_owned(),
            operations: Vec::new(),
            endpoint: "https://node.example".to_owned(),
            last_seen_millis: 0,
        };
        assert!(super::record_matches_signer(&record, &signer.node_id()).is_err());

        record.node_id = signer.node_id();
        assert!(super::record_matches_signer(&record, &signer.node_id()).is_ok());
    }
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --locked modules::control_plane -- --test-threads=1`
Expected: FAIL to compile — `record_matches_signer` is not defined.

- [ ] **Step 3: Carry identity and admission on `AppState`**

In `src/app.rs`, replace the `server_seed`/`server_id` derivation (the `format!`/`blake3::hash` pair) with:

```rust
        let identity = crate::cluster::identity::NodeIdentity::load_or_create(
            &config.paths.state_dir.join(crate::cluster::identity::KEY_FILE),
        )?;
        let server_id = identity.node_id();
        let admission = crate::cluster::admission::build(
            &config.cluster,
            cluster_token.as_deref(),
            &config.paths.state_dir,
        )?;
```

Store `identity` and `admission` on the inner state next to `cluster_token`, and add the two accessors named in **Interfaces**. `capability_manifest` keeps reading `self.inner.server_id`, which is now the public key — the recorded wire-value change.

- [ ] **Step 4: Replace verification in the handlers**

In `src/modules/control_plane.rs`, replace `verify_cluster_request` with:

```rust
fn record_matches_signer(node: &crate::protocol::NodeRecord, signer: &str) -> Result<(), HttpError> {
    if node.node_id != signer {
        return Err(HttpError(crate::error::LiveError::Authentication(
            "cluster node record does not match the signing identity".to_owned(),
        )));
    }
    Ok(())
}

fn authorize_cluster_request(
    state: &AppState,
    headers: &HeaderMap,
    method: &str,
    path: &str,
    query: Option<&str>,
    body: &[u8],
) -> Result<String, HttpError> {
    let proof = crate::cluster::proof::RequestProof {
        node_id: header(headers, crate::cluster::proof::NODE_HEADER)?.to_owned(),
        timestamp: header(headers, crate::cluster::proof::TIMESTAMP_HEADER)?.to_owned(),
        signature: header(headers, crate::cluster::proof::SIGNATURE_HEADER)?.to_owned(),
    };
    // A peer that already knows us signs against our node id. A node joining
    // for the first time cannot: it has not learned our key yet, so it signs
    // against the origin it dialled. Both name *this* node, so neither form
    // replays against a different peer.
    let mut recipients = vec![state.identity().node_id()];
    if let Some(endpoint) = state.config().cluster.advertise_endpoint.as_deref() {
        recipients.push(endpoint.trim_end_matches('/').to_owned());
    }
    let verified = recipients.iter().any(|recipient| {
        crate::cluster::proof::verify_request(&proof, recipient, method, path, query, body).is_ok()
    });
    if !verified {
        return Err(HttpError(crate::error::LiveError::Authentication(
            "invalid cluster request proof".to_owned(),
        )));
    }
    let ticket = headers
        .get(crate::cluster::proof::TICKET_HEADER)
        .and_then(|value| value.to_str().ok());
    match state.admission().authorize(&proof.node_id, ticket) {
        crate::cluster::admission::Decision::Admit => Ok(proof.node_id),
        crate::cluster::admission::Decision::Deny(reason) => Err(HttpError(
            crate::error::LiveError::Authorization(format!("cluster admission denied: {reason}")),
        )),
    }
}
```

Update `join_cluster` to call it with `("POST", crate::cluster::JOIN_PATH, None, &body)` in place of `crate::cluster::verify(...)`, then call `record_matches_signer(&request.node, &signer)?` after `validate_node_record`. Update `list_cluster_objects` and `get_cluster_object` to pass their own method, path, `uri.query()`, and `&[]`. Both need `axum::extract::OriginalUri` added to their signatures to see the raw query.

- [ ] **Step 5: Replace signing on the client side**

In `src/cluster/mod.rs`'s `contact_peer` and `src/cluster/replication.rs`'s `signed_get`, replace the `sign(token, ...)` calls with `proof::sign_request(state.identity(), peer_node_id, method, path, url.query(), body)` and emit the four headers. `contact_peer` and `replicate_peer` both take the peer's `node_id` as a new parameter. The caller has it from the directory entry. For a configured seed never yet contacted it is `None` — the joining node does not know the seed's key yet — and in that case the recipient field is the seed's normalized origin (`https://seed.example:11435`) rather than a node id. The receiver accepts a recipient that equals **either** its own `node_id` **or** its own normalized `cluster.advertise_endpoint`. Binding the origin keeps cross-peer replay protection on the join path, which signing a constant such as `"unknown"` would throw away on precisely the least authenticated request. Add `TICKET_HEADER` carrying `admission::ticket(token, own_node_id)` whenever a cluster token is configured.

- [ ] **Step 6: Delete the superseded code**

Remove `sign`, `verify`, and `SIGNING_CONTEXT` from `src/cluster/mod.rs`, plus their three unit tests. `load_or_create_token` and `TOKEN_FILE` stay — the token is now the admission ticket secret.

- [ ] **Step 7: Run the tests**

Run: `cargo test --workspace --all-targets --locked -- --test-threads=1`
Expected: PASS. `tests/cluster_e2e.rs` must still pass unchanged: both nodes generate distinct keys, and the shared `HOLOGRAM_CLUSTER_E2E_TOKEN` admits each to the other by ticket.

- [ ] **Step 8: Commit**

```bash
git add src/app.rs src/cluster/ src/modules/control_plane.rs
git commit -m "feat(cluster): every request proves which node sent it"
```

---

**Continues in `2026-09-24-distributed-p2p-clustering-phase-1b.md`** — Tasks 6
through 9 (the peer table, ownership and the membership epoch, replication, and
the end-to-end proof), plus the completion criteria. Split only because
`scripts/check-file-size.sh` caps a file at 1500 lines; execute the two in order
as one plan.
