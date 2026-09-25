//! Authenticated seed-based server membership for Hologram nodes.
//!
//! This layer discovers live Hologram frontends, reconciles immutable
//! content, and deliberately does not retry an in-flight mutable mutation
//! against another authority.

pub(crate) mod admission;
pub(crate) mod identity;
pub(crate) mod proof;
mod replication;

use crate::app::AppState;
use crate::config::validate_cluster_endpoint;
use crate::error::{LiveError, Result};
use crate::protocol::{ClusterJoinRequest, ClusterJoinResponse, NodeRecord};
use crate::util::now_millis;
use replication::replicate_peer;
use std::collections::BTreeSet;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
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
    let mut peers: BTreeSet<String> = config
        .seeds
        .iter()
        .map(|endpoint| normalize_endpoint(endpoint))
        .filter(|endpoint| endpoint != &self_endpoint)
        .collect();
    let mut ticker = tokio::time::interval(Duration::from_secs(config.heartbeat_interval_secs));

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                if let Err(error) = state.nodes().heartbeat(self_node.clone()) {
                    tracing::warn!(%error, "failed to persist local cluster heartbeat");
                }

                let mut joins = JoinSet::new();
                for endpoint in peers.iter().take(config.max_peers).cloned() {
                    let state = state.clone();
                    let client = client.clone();
                    let token = token.clone();
                    let node = self_node.clone();
                    joins.spawn(async move {
                        let result =
                            contact_peer(&state, &client, &endpoint, &token, node).await;
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
                            if response.node.node_id != self_node.node_id {
                                if let Err(error) = state.nodes().heartbeat(response.node.clone()) {
                                    tracing::warn!(%error, peer = %endpoint, "failed to persist peer heartbeat");
                                }
                            }
                            if let Err(error) =
                                replicate_peer(&state, &client, &endpoint, &token).await
                            {
                                tracing::debug!(%error, peer = %endpoint, "cluster immutable-content replication failed");
                            }
                            for peer in response.peers {
                                if peers.len() >= config.max_peers {
                                    break;
                                }
                                if peer.node_id == self_node.node_id || peer.endpoint.is_empty() {
                                    continue;
                                }
                                if validate_cluster_endpoint(&peer.endpoint).is_ok() {
                                    let endpoint = normalize_endpoint(&peer.endpoint);
                                    if endpoint != self_endpoint {
                                        peers.insert(endpoint);
                                    }
                                }
                            }
                        }
                        Err(error) => {
                            tracing::debug!(%error, peer = %endpoint, "cluster peer is unavailable");
                        }
                    }
                }

                let cutoff = now_millis().saturating_sub(config.node_ttl_secs.saturating_mul(1000));
                match state.nodes().prune_older_than(cutoff, &self_node.node_id) {
                    Ok(removed) if removed > 0 => tracing::info!(removed, "pruned stale cluster members"),
                    Ok(_) => {}
                    Err(error) => tracing::warn!(%error, "failed to prune stale cluster members"),
                }
            }
            () = state.wait_shutdown() => break,
        }
    }
}

async fn contact_peer(
    state: &AppState,
    client: &reqwest::Client,
    endpoint: &str,
    token: &str,
    node: NodeRecord,
) -> Result<ClusterJoinResponse> {
    let mut url = reqwest::Url::parse(endpoint)
        .map_err(|error| LiveError::Config(format!("invalid cluster endpoint: {error}")))?;
    url.set_path(JOIN_PATH);
    let body = serde_json::to_vec(&ClusterJoinRequest { node })?;
    let recipient = recipient_for(endpoint);
    // `url.path()` is `JOIN_PATH` and there is no query: both are values this
    // crate owns, and `Url` percent-encodes anything that could pass for the
    // preimage's newline separator.
    let request_proof = proof::sign_request(
        state.identity(),
        &recipient,
        "POST",
        url.path(),
        url.query(),
        &body,
    );
    let response = sign_headers(client.post(url), &request_proof, token)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body)
        .send()
        .await
        .map_err(|error| LiveError::Transport(format!("join cluster peer {endpoint}: {error}")))?;
    if !response.status().is_success() {
        return Err(LiveError::Transport(format!(
            "join cluster peer {endpoint}: HTTP {}",
            response.status()
        )));
    }
    let mut response = response;
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| LiveError::Transport(format!("read cluster peer {endpoint}: {error}")))?
    {
        if body.len().saturating_add(chunk.len()) > MAX_JOIN_BYTES {
            return Err(LiveError::Protocol(format!(
                "cluster peer {endpoint} response exceeds {MAX_JOIN_BYTES} bytes"
            )));
        }
        body.extend_from_slice(&chunk);
    }
    let response: ClusterJoinResponse = serde_json::from_slice(&body).map_err(|error| {
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
/// id (`control_plane::proof_names_self`), because Phase 2's iroh addresses
/// *are* node ids and have no origin.
fn recipient_for(endpoint: &str) -> String {
    normalize_endpoint(endpoint)
}

/// Attaches the per-node proof and the admission ticket for *our* identity.
/// The ticket is derived from the shared token and our node id, so it admits
/// this node and no other even if it is captured in flight.
fn sign_headers(
    request: reqwest::RequestBuilder,
    request_proof: &proof::RequestProof,
    token: &str,
) -> reqwest::RequestBuilder {
    request
        .header(proof::NODE_HEADER, &request_proof.node_id)
        .header(proof::TIMESTAMP_HEADER, &request_proof.timestamp)
        .header(proof::SIGNATURE_HEADER, &request_proof.signature)
        .header(
            proof::TICKET_HEADER,
            admission::ticket(token, &request_proof.node_id),
        )
}

fn normalize_endpoint(endpoint: &str) -> String {
    endpoint.trim_end_matches('/').to_owned()
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
}
