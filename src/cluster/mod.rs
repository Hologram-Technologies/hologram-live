//! Authenticated seed-based server membership for Hologram nodes.
//!
//! This layer discovers live Hologram frontends, reconciles immutable
//! content, and deliberately does not retry an in-flight mutable mutation
//! against another authority.

pub(crate) mod admission;
pub(crate) mod identity;
mod membership;
pub(crate) mod network;
pub(crate) mod proof;
mod replication;

use crate::app::AppState;
use crate::config::validate_cluster_endpoint;
use crate::error::{LiveError, Result};
use crate::protocol::{ClusterJoinRequest, ClusterJoinResponse, NodeRecord};
use crate::util::now_millis;
use membership::PeerTable;
use network::{ClusterRequest, HttpNetwork, NetworkRegistry};
use replication::replicate_peer;
use std::collections::BTreeSet;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::task::{JoinHandle, JoinSet};

pub const JOIN_PATH: &str = "/api/v1/cluster/join";
pub const OBJECTS_PATH: &str = "/api/v1/cluster/objects";
pub const OBJECT_PATH: &str = "/api/v1/cluster/objects/{id}";
pub const MAX_JOIN_BYTES: usize = 1024 * 1024;
pub const TOKEN_FILE: &str = "cluster.token";

pub fn load_or_create_token(config: &crate::config::AppConfig) -> Result<String> {
    if let Some(token) = config.cluster_token() {
        validate_token(&token, &config.cluster.token_env)?;
        return Ok(token);
    }
    load_or_create_token_file(&config.paths.state_dir.join(TOKEN_FILE))
}

fn load_or_create_token_file(path: &Path) -> Result<String> {
    match std::fs::read_to_string(path) {
        Ok(token) => {
            secure_token_file(path)?;
            let token = token.trim().to_owned();
            validate_token(&token, &path.display().to_string())?;
            Ok(token)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => create_token_file(path),
        Err(error) => Err(LiveError::io(path, error)),
    }
}

fn create_token_file(path: &Path) -> Result<String> {
    let mut random = [0_u8; 32];
    getrandom::fill(&mut random)
        .map_err(|error| LiveError::Io(format!("generate cluster token: {error}")))?;
    let token = crate::util::hex(&random);

    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(path) {
        Ok(mut file) => {
            file.write_all(token.as_bytes())
                .and_then(|()| file.write_all(b"\n"))
                .and_then(|()| file.sync_all())
                .map_err(|error| LiveError::io(path, error))?;
            Ok(token)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            load_or_create_token_file(path)
        }
        Err(error) => Err(LiveError::io(path, error)),
    }
}

fn validate_token(token: &str, source: &str) -> Result<()> {
    if token.len() < 32 {
        return Err(LiveError::Config(format!(
            "cluster token from {source} must contain at least 32 bytes"
        )));
    }
    Ok(())
}

#[cfg(unix)]
fn secure_token_file(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let metadata = std::fs::metadata(path).map_err(|error| LiveError::io(path, error))?;
    if metadata.permissions().mode() & 0o077 != 0 {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(|error| LiveError::io(path, error))?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn secure_token_file(_path: &Path) -> Result<()> {
    Ok(())
}

pub fn spawn(state: AppState) -> Option<JoinHandle<()>> {
    state.config().cluster.advertise_endpoint.as_ref()?;
    Some(tokio::spawn(run(state)))
}

async fn run(state: AppState) {
    let config = state.config().cluster.clone();
    let self_endpoint = config
        .advertise_endpoint
        .as_deref()
        .map(normalize_endpoint)
        .expect("cluster task requires an advertised endpoint");
    let self_node = state.local_node_record(self_endpoint.clone());
    let Some(token) = state.cluster_token().map(str::to_owned) else {
        tracing::error!("cluster token disappeared after configuration validation");
        return;
    };
    crate::util::install_crypto_provider();
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(config.request_timeout_secs))
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            tracing::error!(%error, "failed to build cluster membership client");
            return;
        }
    };
    // One network today, and the call sites below name none of them: every
    // request goes out through the registry, which routes on the address's own
    // scheme. Phase 2 pushes an iroh network into this same vector.
    let networks = Arc::new(NetworkRegistry::new(vec![Arc::new(
        HttpNetwork::new(client).with_advertised(Some(self_endpoint.clone())),
    )]));
    tracing::debug!(
        addresses = ?networks.local_addresses(),
        "cluster networks are ready"
    );
    let seeds: Vec<String> = config
        .seeds
        .iter()
        .map(|endpoint| normalize_endpoint(endpoint))
        .filter(|endpoint| endpoint != &self_endpoint)
        .collect();
    let mut table = PeerTable::new(seeds);
    table.seed_from_directory(
        &state.nodes().list().unwrap_or_default(),
        &self_endpoint,
        config.max_peers,
    );
    // The dial list, recovered once. Without it a node restarted with no
    // configured seeds has nowhere to knock: its directory is written only by
    // authenticated inbound joins, so until someone dials it, it holds only
    // itself. See `load_dialled` for why an endpoint may be recovered this way
    // when a join reply's record may not.
    let dialled_path = state
        .config()
        .paths
        .state_dir
        .join(membership::DIALLED_FILE);
    let mut dialled: BTreeSet<String> = load_dialled(&dialled_path, &self_endpoint)
        .into_iter()
        .collect();
    table.seed_from_endpoints(
        &dialled.iter().cloned().collect::<Vec<String>>(),
        &self_endpoint,
        config.max_peers,
    );
    let backoff_ceiling_millis = config.node_ttl_secs.saturating_mul(1000);
    // Anti-entropy is decoupled from the heartbeat cadence and tracked per
    // peer (`PeerTable::replication_due`/`record_replication`), not as one
    // global timer: a global gate combined with `due()`'s fixed-size
    // rotation cursor phase-locks replication to whichever batch of peers
    // happens to be due for contact on a round that is also due for
    // replication, which can starve most peers forever rather than merely
    // delaying them (see the comment on `PeerState::last_replicated_millis`
    // in `membership.rs`).
    let replication_interval_millis = config.replication_interval_secs.saturating_mul(1000);
    let mut ticker = tokio::time::interval(Duration::from_secs(config.heartbeat_interval_secs));
    // Replication now runs inline in this loop, so one slow peer can make a
    // round overrun `heartbeat_interval_secs`. The default `Burst` behaviour
    // would then fire every missed tick back to back, turning an overrun into
    // a flurry of catch-up rounds against the peers that were already slow.
    // `Delay` simply resumes the cadence from now, matching `tls.rs`,
    // `oci/metrics.rs` and `oci/debug.rs`.
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                if let Err(error) = state.nodes().heartbeat(self_node.clone()) {
                    tracing::warn!(%error, "failed to persist local cluster heartbeat");
                }

                // Re-seeds every round, not just once at startup. This is
                // what makes admission symmetric: a joiner authenticates to
                // the seed with a signed, ticket-bearing request, but a join
                // *response* carries no signature, so nothing analogous
                // authenticates the seed back — unless the seed also becomes
                // a caller. The seed's directory gains the joiner from that
                // authenticated inbound join; re-reading it here is what lets
                // the seed notice and dial the joiner in a later round, at
                // which point `contact_peer` presents a real ticket
                // (`signed_request` sets `ClusterRequest::ticket`, which the
                // network puts on the wire) and the joiner admits the seed the
                // same way any node is admitted —
                // proof of holding the shared token, not trust in an
                // unsigned response.
                //
                // Cheap, and bounded: this is one directory read per round
                // (`NodeDirectory` itself is not capped at `max_peers` — it
                // grows with every inbound join), but `seed_from_directory`
                // stops *inserting* once the table reaches `max_peers`, the
                // same bound the gossiped-peer path below already enforces.
                // `insert` is also a no-op for an endpoint already in the
                // table, so a stable membership costs one cheap directory
                // read and no table churn.
                table.seed_from_directory(&state.nodes().list().unwrap_or_default(), &self_endpoint, config.max_peers);

                let round_started_millis = now_millis();
                let mut dialled_changed = false;
                let mut joins = JoinSet::new();
                for endpoint in table.due(round_started_millis, config.fanout) {
                    let state = state.clone();
                    let networks = networks.clone();
                    let token = token.clone();
                    let node = self_node.clone();
                    joins.spawn(async move {
                        let result =
                            contact_peer(&state, &networks, &endpoint, &token, node).await;
                        (endpoint, result)
                    });
                }

                while let Some(joined) = joins.join_next().await {
                    let (endpoint, result) = match joined {
                        Ok(result) => result,
                        Err(error) => {
                            tracing::warn!(%error, "cluster join task failed");
                            continue;
                        }
                    };
                    match result {
                        Ok(response) => {
                            table.record_success(&endpoint);
                            // An origin that answered a join is worth knocking
                            // on again after a restart. Bounded by the same
                            // `max_peers` the table is.
                            if dialled.len() < config.max_peers
                                && dialled.insert(endpoint.clone())
                            {
                                dialled_changed = true;
                            }
                            // Deliberately does **not** persist `response.node`.
                            // A join reply is unsigned, so its `node_id` is the
                            // responder's unproven claim while `endpoint` and
                            // `operations` are whatever it chose to send. The
                            // directory is keyed by `node_id` and a write
                            // replaces the whole record, so persisting a reply
                            // would let any origin this node dials install an
                            // *already-admitted* member's `node_id` against its
                            // own endpoint — and `cluster_owner` intersects the
                            // directory with the admitted set, so
                            // `/api/v1/nodes/owner` and `/api/v1/nodes/placement`
                            // would then hand out the attacker's origin,
                            // including for inference and Holo placement. Only
                            // `control_plane::join_cluster` writes peer records,
                            // where `record_matches_signer` has proved the
                            // record's `node_id` is the signer's. Mutual dialling
                            // makes that sufficient: `seed_from_directory` runs
                            // every round, so each side comes to dial the other
                            // and each learns the other from an authenticated
                            // inbound join.
                            if table.replication_due(&endpoint, now_millis(), replication_interval_millis) {
                                if let Err(error) =
                                    replicate_peer(&state, &networks, &endpoint, &token).await
                                {
                                    tracing::debug!(%error, peer = %endpoint, "cluster immutable-content replication failed");
                                }
                                table.record_replication(&endpoint, now_millis());
                            }
                            for peer in response.peers {
                                if table.len() >= config.max_peers {
                                    break;
                                }
                                if peer.node_id == self_node.node_id || peer.endpoint.is_empty() {
                                    continue;
                                }
                                if validate_cluster_endpoint(&peer.endpoint).is_ok() {
                                    let endpoint = normalize_endpoint(&peer.endpoint);
                                    if endpoint != self_endpoint {
                                        table.insert(endpoint);
                                    }
                                }
                            }
                        }
                        Err(error) => {
                            tracing::debug!(%error, peer = %endpoint, "cluster peer is unavailable");
                            table.record_failure(&endpoint, now_millis(), backoff_ceiling_millis);
                        }
                    }
                }

                let cutoff = now_millis().saturating_sub(config.node_ttl_secs.saturating_mul(1000));
                let before_prune = state.nodes().list().unwrap_or_default();
                match state.nodes().prune_older_than(cutoff, &self_node.node_id) {
                    Ok(removed) if removed > 0 => {
                        tracing::info!(removed, "pruned stale cluster members");
                        let after_prune = state.nodes().list().unwrap_or_default();
                        dialled_changed |= evict_pruned(
                            &mut table,
                            &mut dialled,
                            &before_prune,
                            &after_prune,
                        );
                    }
                    Ok(_) => {}
                    Err(error) => tracing::warn!(%error, "failed to prune stale cluster members"),
                }

                if dialled_changed {
                    store_dialled(&dialled_path, &dialled);
                }
            }
            () = state.wait_shutdown() => break,
        }
    }
}

async fn contact_peer(
    state: &AppState,
    networks: &NetworkRegistry,
    endpoint: &str,
    token: &str,
    node: NodeRecord,
) -> Result<ClusterJoinResponse> {
    let mut url = reqwest::Url::parse(endpoint)
        .map_err(|error| LiveError::Config(format!("invalid cluster endpoint: {error}")))?;
    url.set_path(JOIN_PATH);
    let body = serde_json::to_vec(&ClusterJoinRequest { node })?;
    let recipient = recipient_for(endpoint);
    proof::reject_separators("POST", url.path(), url.query(), &recipient)?;
    let request_proof = proof::sign_request(
        state.identity(),
        &recipient,
        "POST",
        url.path(),
        url.query(),
        &body,
    );
    // Reports this node's membership epoch on the wire (see
    // `proof::EPOCH_HEADER`). The receiver does not currently refuse on a
    // mismatch. `run` re-seeds the peer table from the node directory every
    // round (not just once at startup), so a seed comes to dial the joiner
    // back and both directions present a real, ticket-proven admission — the
    // two sides' admitted sets do converge. But convergence is not atomic:
    // it takes the seed's next round to notice a newly-directoried peer and
    // one more round trip to be admitted by it, so two nodes queried in that
    // window can legitimately disagree for a beat. Enforcing equality safely
    // would need sender-side refresh-and-retry on a mismatch, which is more
    // than this phase carries; the header stays observability-only until
    // that lands.
    let epoch = crate::ownership::epoch(&state.admitted_with_self());
    // The join bound, now enforced while the answer is read (see
    // `ClusterNetwork::send`) rather than after it is all in memory. The check
    // below still stands as the backstop for a network that does not honour it.
    let mut request = signed_request(
        "POST",
        &url,
        body,
        &recipient,
        &request_proof,
        token,
        Some(MAX_JOIN_BYTES as u64),
    );
    request.epoch = Some(epoch);
    let response = networks.send(endpoint, request).await?;
    if !response.is_success() {
        return Err(LiveError::Transport(format!(
            "join cluster peer {endpoint}: HTTP {}",
            response.status
        )));
    }
    // Unreachable over HTTP, which refuses an oversize body mid-read: this is
    // the backstop for a network implementation that ignores
    // `max_response_bytes`, and it keeps this bound stated where a reader of
    // the join path can see it.
    if response.body.len() > MAX_JOIN_BYTES {
        return Err(LiveError::Protocol(format!(
            "cluster peer {endpoint} response exceeds {MAX_JOIN_BYTES} bytes"
        )));
    }
    let response: ClusterJoinResponse =
        serde_json::from_slice(&response.body).map_err(|error| {
            LiveError::Protocol(format!("decode cluster peer {endpoint} response: {error}"))
        })?;
    validate_node_record(&response.node)?;
    Ok(response)
}

/// The recipient field a request to `endpoint` must be signed against: always
/// the normalized origin about to be dialled, never a node id.
///
/// A node id is only ever learned from another peer's unsigned response, so it
/// is that peer's *claim* about a third party. Binding it would let an
/// admitted-but-rogue peer report a victim's `node_id` against an endpoint the
/// rogue controls, collect the proof we then mint for the victim, and replay it
/// against the real victim. Under `admission = "allowlist"` with asymmetric
/// `trusted_keys` the victim may trust us and refuse the rogue, so that is
/// privilege escalation across a security boundary, not a no-op.
///
/// The origin defends replay exactly as well — a proof names the one endpoint
/// it was minted for, and no other node accepts it — while depending on no
/// peer's honesty about anyone else. The receiving half still accepts a node
/// id (`control_plane::recipient_names_self`), because Phase 2's iroh addresses
/// *are* node ids and have no origin.
fn recipient_for(endpoint: &str) -> String {
    normalize_endpoint(endpoint)
}

/// Whether two endpoint spellings name the same origin.
///
/// Origin-only binding would otherwise be brittle in a way an operator cannot
/// diagnose: a seed configured as `https://Seed.Example:443/` names the same
/// server as an `advertise_endpoint` of `https://seed.example`, but the raw
/// strings differ, so every request from that peer would fail authentication
/// forever. Comparing the *parsed* origin — scheme, lowercased host, and the
/// port with the scheme default made explicit — reconciles host case, implicit
/// versus explicit default port, the IPv6 bracket forms that `Url` canonicalizes
/// on parse, and the trailing slash.
///
/// It stays exact on those three components. There is deliberately no substring
/// or prefix fallback, and an IP address is *not* reconciled with a DNS name
/// that resolves to it: that is a genuinely different origin, and failing is
/// the correct answer.
pub(crate) fn same_origin(left: &str, right: &str) -> bool {
    match (origin_parts(left), origin_parts(right)) {
        (Some(left), Some(right)) => left == right,
        // A value with no host — a node id, or anything unparsable — has no
        // origin to compare, so it can only match by exact equality elsewhere.
        _ => false,
    }
}

fn origin_parts(endpoint: &str) -> Option<(String, String, u16)> {
    let url = reqwest::Url::parse(endpoint).ok()?;
    Some((
        url.scheme().to_ascii_lowercase(),
        url.host_str()?.to_ascii_lowercase(),
        url.port_or_known_default()?,
    ))
}

/// Carries the per-node proof and the admission ticket for *our* identity.
/// The ticket is derived from the shared token and our node id, so it admits
/// this node and no other even if it is captured in flight.
///
/// This is the whole of what used to be a set of HTTP headers, now data on a
/// [`ClusterRequest`]: a network decides how to put it on the wire and never
/// what it says. `recipient` in particular arrives already bound into
/// `request_proof`'s signature, so no network can re-address the request, and
/// `max_response_bytes` travels the same way so no network can decide to accept
/// a larger answer than the caller asked for.
fn signed_request(
    method: &'static str,
    url: &reqwest::Url,
    body: Vec<u8>,
    recipient: &str,
    request_proof: &proof::RequestProof,
    token: &str,
    max_response_bytes: Option<u64>,
) -> ClusterRequest {
    ClusterRequest {
        method,
        path: url.path().to_owned(),
        query: url.query().map(str::to_owned),
        body,
        recipient: recipient.to_owned(),
        proof: request_proof.clone(),
        ticket: Some(admission::ticket(token, &request_proof.node_id)),
        // Informational, and set by the one caller that reports it; the
        // ceiling is an argument rather than a field left at a default,
        // because a forgotten bound is not a safe bound.
        epoch: None,
        max_response_bytes,
    }
}

fn normalize_endpoint(endpoint: &str) -> String {
    endpoint.trim_end_matches('/').to_owned()
}

/// The origins this node has dialled and been answered by, recovered from
/// `paths.state_dir/cluster-peers.json`.
///
/// A *dial list*, and deliberately nothing more. `run` no longer persists a
/// join reply's `NodeRecord` — a reply is unsigned, so its `node_id` is the
/// responder's unproven claim about itself and the rest of the record is
/// whatever it chose to send — which means a node learns a peer's record only
/// from that peer's authenticated inbound join. That is the correct rule, and
/// it leaves one gap: a node restarted with **no** configured seeds has a
/// directory holding only itself, so it has no address to knock on and has to
/// wait to be dialled. This file closes exactly that gap and nothing else.
///
/// What it is not: it is not a record, so it cannot make an origin an
/// ownership candidate (`cluster_owner` intersects the *directory* with the
/// admitted set), it cannot admit anyone (admission still needs a valid
/// ticket on an authenticated inbound request), and it asserts nothing about
/// who answers at that origin. Every entry is also an origin this node
/// already chose to dial, so a restart gains no reachable claim it did not
/// have before it.
///
/// A file that is missing, unreadable or malformed is simply no dial list:
/// this is a recovery hint, and failing the cluster task over it would be
/// worse than starting with the configured seeds alone. Entries are screened
/// with `validate_cluster_endpoint` so a tampered file cannot smuggle a value
/// the configured-seed path would have refused.
fn load_dialled(path: &Path, self_endpoint: &str) -> Vec<String> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(error) => {
            tracing::debug!(%error, path = %path.display(), "no cluster dial list to recover");
            return Vec::new();
        }
    };
    let endpoints: Vec<String> = match serde_json::from_slice(&bytes) {
        Ok(endpoints) => endpoints,
        Err(error) => {
            tracing::warn!(%error, path = %path.display(), "ignoring an unreadable cluster dial list");
            return Vec::new();
        }
    };
    endpoints
        .into_iter()
        .filter(|endpoint| validate_cluster_endpoint(endpoint).is_ok())
        .map(|endpoint| normalize_endpoint(&endpoint))
        .filter(|endpoint| endpoint != self_endpoint)
        .collect()
}

/// Rewrites the dial list. A failure is logged and otherwise ignored: the
/// running cluster does not depend on it, only the next restart's head start
/// does.
fn store_dialled(path: &Path, endpoints: &BTreeSet<String>) {
    match serde_json::to_vec_pretty(endpoints)
        .map_err(LiveError::from)
        .and_then(|bytes| crate::util::atomic_write(path, &bytes))
    {
        Ok(()) => {}
        Err(error) => tracing::warn!(%error, "failed to persist the cluster dial list"),
    }
}

/// Endpoints that pruning actually orphaned: named by a `before` record whose
/// node id did not survive pruning, and not claimed by any surviving
/// record's endpoint either.
///
/// The directory is keyed by node id, not endpoint. A peer that rotates its
/// identity while keeping the same `advertise_endpoint` ages its old node id
/// out of the directory while a new node id with that same endpoint keeps
/// heartbeating; diffing node ids alone would evict a still-live peer for at
/// least a round. Checking the surviving endpoints too keeps that peer in
/// the table.
fn prune_evictions(before: &[NodeRecord], after: &[NodeRecord]) -> Vec<String> {
    let surviving_ids: BTreeSet<&str> = after.iter().map(|node| node.node_id.as_str()).collect();
    let surviving_endpoints: BTreeSet<String> = after
        .iter()
        .map(|node| normalize_endpoint(&node.endpoint))
        .collect();
    before
        .iter()
        .filter(|node| !surviving_ids.contains(node.node_id.as_str()))
        .map(|node| normalize_endpoint(&node.endpoint))
        .filter(|endpoint| !surviving_endpoints.contains(endpoint))
        .collect()
}

/// Drops the peers that pruning actually orphaned from the working set: out of
/// the peer table, so a dead endpoint stops burning a `fanout` slot, and out of
/// the dial list, so it stops being worth a restart's first knock. Reports
/// whether the dial list changed, which is when it needs rewriting.
///
/// Extracted from `run` so the wiring between `prune_evictions` and
/// `PeerTable::evict` is testable. `prune_evictions` was covered on its own,
/// and `PeerTable::evict` was too, but nothing joined them: eviction could be
/// wired to the wrong list, or not wired at all, and every test still passed.
fn evict_pruned(
    table: &mut PeerTable,
    dialled: &mut BTreeSet<String>,
    before: &[NodeRecord],
    after: &[NodeRecord],
) -> bool {
    let mut dialled_changed = false;
    for endpoint in prune_evictions(before, after) {
        table.evict(&endpoint);
        if dialled.remove(&endpoint) {
            dialled_changed = true;
        }
    }
    dialled_changed
}

pub fn validate_node_record(node: &NodeRecord) -> Result<()> {
    if node.node_id.is_empty() || node.node_id.len() > 256 {
        return Err(LiveError::Protocol(
            "cluster node_id must contain 1 to 256 bytes".to_owned(),
        ));
    }
    if node.version.len() > 128 {
        return Err(LiveError::Protocol(
            "cluster node version exceeds 128 bytes".to_owned(),
        ));
    }
    if node.operations.len() > 1024
        || node
            .operations
            .iter()
            .any(|operation| operation.len() > 256)
    {
        return Err(LiveError::Protocol(
            "cluster node advertises invalid operations".to_owned(),
        ));
    }
    validate_cluster_endpoint(&node.endpoint)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_cluster_token_is_private_and_reused() {
        let directory = tempfile::tempdir().expect("temporary state directory");
        let path = directory.path().join(TOKEN_FILE);

        let first = load_or_create_token_file(&path).expect("generate token");
        let second = load_or_create_token_file(&path).expect("reload token");

        assert_eq!(first, second);
        assert_eq!(first.len(), 64);
        assert!(first.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_eq!(
            std::fs::read_to_string(&path).expect("read token file"),
            format!("{first}\n")
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path)
                .expect("token metadata")
                .permissions()
                .mode();
            assert_eq!(mode & 0o077, 0);
        }
    }

    #[cfg(unix)]
    #[test]
    fn existing_cluster_token_permissions_are_hardened() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().expect("temporary state directory");
        let path = directory.path().join(TOKEN_FILE);
        let token = "a".repeat(64);
        std::fs::write(&path, &token).expect("write token");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
            .expect("make token readable");

        assert_eq!(load_or_create_token_file(&path).expect("load token"), token);
        let mode = std::fs::metadata(&path)
            .expect("token metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0);
    }

    // Fix round 2, finding A: differing spellings of one origin must reconcile,
    // so that origin-only binding does not silently refuse a correctly
    // configured peer forever. A genuinely different host must still fail.
    #[test]
    fn spellings_of_one_origin_match_and_different_origins_do_not() {
        // Host case.
        assert!(same_origin(
            "https://Seed.Example:11435",
            "https://seed.example:11435"
        ));
        // Implicit versus explicit default port, per scheme.
        assert!(same_origin(
            "https://seed.example",
            "https://seed.example:443"
        ));
        assert!(same_origin("http://seed.example", "http://seed.example:80"));
        // Trailing slash, and all three together.
        assert!(same_origin(
            "https://SEED.example/",
            "https://seed.example:443"
        ));
        // IPv6 bracket forms canonicalized by the parser.
        assert!(same_origin(
            "http://[0:0:0:0:0:0:0:1]:11435",
            "http://[::1]:11435"
        ));

        // Genuinely different origins, including the cases that must not be
        // reconciled by any fallback.
        assert!(!same_origin(
            "https://seed.example:11435",
            "https://other.example:11435"
        ));
        assert!(!same_origin(
            "https://127.0.0.1:11435",
            "https://localhost:11435"
        ));
        assert!(!same_origin("https://seed.example", "http://seed.example"));
        assert!(!same_origin(
            "https://seed.example:11435",
            "https://seed.example:11436"
        ));
        // A prefix is not a match.
        assert!(!same_origin(
            "https://seed.example.evil.test",
            "https://seed.example"
        ));
        // A node id has no origin, so it never matches one.
        assert!(!same_origin(
            "ed25519:d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a",
            "https://seed.example"
        ));
    }

    // Fix round 1: outbound proofs bind the origin being dialled and nothing
    // else. There is no node-id branch to take, so no peer's claim about a
    // third party can steer what we sign.
    #[test]
    fn the_outbound_recipient_is_always_the_normalized_origin() {
        assert_eq!(
            recipient_for("https://seed.example:11435"),
            "https://seed.example:11435"
        );
        assert_eq!(
            recipient_for("https://seed.example:11435/"),
            "https://seed.example:11435"
        );
    }

    fn node(node_id: &str, endpoint: &str) -> NodeRecord {
        NodeRecord {
            node_id: node_id.to_owned(),
            version: "test".to_owned(),
            operations: Vec::new(),
            endpoint: endpoint.to_owned(),
            last_seen_millis: 0,
        }
    }

    // Fix round 1, finding 2: a genuinely stale peer, gone from the
    // directory under any node id, must still be evicted.
    #[test]
    fn a_peer_absent_from_every_surviving_record_is_evicted() {
        let before = vec![
            node("ed25519:self", "https://self.example"),
            node("ed25519:gone", "https://gone.example"),
        ];
        let after = vec![node("ed25519:self", "https://self.example")];
        assert_eq!(
            prune_evictions(&before, &after),
            vec!["https://gone.example".to_owned()]
        );
    }

    // Fix round 1, finding 2: the directory is keyed by node id, not
    // endpoint. A peer that rotates identity while keeping the same
    // advertise_endpoint must not be evicted just because its old node id
    // aged out — a new node id with that same endpoint is still heartbeating.
    #[test]
    fn a_peer_that_rotated_identity_is_not_evicted() {
        let before = vec![node("ed25519:old", "https://rotated.example")];
        let after = vec![node("ed25519:new", "https://rotated.example")];
        assert!(prune_evictions(&before, &after).is_empty());
    }

    // Final review, FIX 6: `prune_evictions` was tested, `PeerTable::evict` was
    // tested, and the wiring between them was not. A pruned peer that stayed in
    // the table burns a `fanout` slot forever, and
    // `a_restarted_node_rejoins_without_any_configured_seed` passes with
    // eviction broken — it only waits for the *directory* to shrink, which
    // `prune_older_than` does on its own.
    #[test]
    fn pruning_the_directory_drops_the_peer_from_the_table_and_the_dial_list() {
        let mut table = PeerTable::new(Vec::new());
        table.insert("https://gone.example".to_owned());
        table.insert("https://live.example".to_owned());
        let mut dialled: BTreeSet<String> = ["https://gone.example", "https://live.example"]
            .into_iter()
            .map(str::to_owned)
            .collect();

        let before = vec![
            node("ed25519:self", "https://self.example"),
            node("ed25519:gone", "https://gone.example"),
            node("ed25519:live", "https://live.example"),
        ];
        let after = vec![
            node("ed25519:self", "https://self.example"),
            node("ed25519:live", "https://live.example"),
        ];

        assert!(
            evict_pruned(&mut table, &mut dialled, &before, &after),
            "dropping an endpoint from the dial list must ask for a rewrite"
        );
        assert_eq!(
            table.due(0, 8),
            vec!["https://live.example".to_owned()],
            "the pruned peer must leave the table and the surviving one must stay"
        );
        assert_eq!(
            dialled,
            ["https://live.example".to_owned()].into_iter().collect(),
            "the pruned peer must leave the dial list too"
        );
    }

    // The other half: a peer that only rotated its node id is still live, so
    // the wiring must not evict it. Without this, "evict everything in
    // `before`" would pass the test above.
    #[test]
    fn pruning_an_identity_rotation_evicts_nothing() {
        let mut table = PeerTable::new(Vec::new());
        table.insert("https://rotated.example".to_owned());
        let mut dialled: BTreeSet<String> =
            ["https://rotated.example".to_owned()].into_iter().collect();

        let before = vec![node("ed25519:old", "https://rotated.example")];
        let after = vec![node("ed25519:new", "https://rotated.example")];

        assert!(!evict_pruned(&mut table, &mut dialled, &before, &after));
        assert_eq!(table.due(0, 8), vec!["https://rotated.example".to_owned()]);
        assert_eq!(dialled.len(), 1);
    }

    // Final review, FIX 1: the dial list is endpoints and nothing else, and a
    // value the configured-seed path would have refused must not slip in
    // through the file. Self is filtered because a node must never seed itself
    // as a peer (`seed_from_directory` filters it for the same reason).
    #[test]
    fn the_dial_list_only_recovers_endpoints_a_seed_could_have_been() {
        let directory = tempfile::tempdir().expect("temporary state directory");
        let path = directory.path().join(membership::DIALLED_FILE);
        std::fs::write(
            &path,
            serde_json::to_vec(&[
                "https://peer.example",
                "https://self.example",
                // Refused by `validate_cluster_endpoint`: plaintext to a
                // non-loopback host, a credential, a path, and a value that is
                // not a URL at all.
                "http://public.example",
                "https://user:pass@peer.example",
                "https://peer.example/admin",
                "ed25519:aa",
            ])
            .expect("encode a dial list"),
        )
        .expect("write a dial list");

        assert_eq!(
            load_dialled(&path, "https://self.example"),
            vec!["https://peer.example".to_owned()]
        );

        // A missing or unreadable file is simply no dial list, never a failure.
        assert!(load_dialled(
            &directory.path().join("absent.json"),
            "https://self.example"
        )
        .is_empty());
        std::fs::write(&path, b"not json").expect("corrupt the dial list");
        assert!(load_dialled(&path, "https://self.example").is_empty());
    }

    // And a round trip, since `store_dialled` is what the next start reads.
    #[test]
    fn a_stored_dial_list_is_recovered_verbatim() {
        let directory = tempfile::tempdir().expect("temporary state directory");
        let path = directory.path().join(membership::DIALLED_FILE);
        let dialled: BTreeSet<String> = ["https://a.example", "https://b.example"]
            .into_iter()
            .map(str::to_owned)
            .collect();
        store_dialled(&path, &dialled);
        assert_eq!(
            load_dialled(&path, "https://self.example"),
            vec![
                "https://a.example".to_owned(),
                "https://b.example".to_owned()
            ]
        );
    }
}
