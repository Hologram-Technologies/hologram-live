//! Authenticated seed-based server membership for Hologram nodes.
//!
//! This layer discovers live Hologram frontends, reconciles immutable
//! content, and deliberately does not retry an in-flight mutable mutation
//! against another authority.

use crate::app::AppState;
use crate::config::validate_cluster_endpoint;
use crate::error::{LiveError, Result};
use crate::protocol::{
    ClusterJoinRequest, ClusterJoinResponse, NodeRecord, ObjectPage, ObjectQuery,
};
use crate::util::{constant_time_eq, now_millis};
use std::collections::BTreeSet;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::time::Duration;
use tokio::task::{JoinHandle, JoinSet};

pub const JOIN_PATH: &str = "/api/v1/cluster/join";
pub const OBJECTS_PATH: &str = "/api/v1/cluster/objects";
pub const OBJECT_PATH: &str = "/api/v1/cluster/objects/{id}";
pub const TIMESTAMP_HEADER: &str = "x-hologram-cluster-timestamp";
pub const SIGNATURE_HEADER: &str = "x-hologram-cluster-signature";
const SIGNING_CONTEXT: &str = "dev.hologram.live.cluster-join.v1";
const MAX_CLOCK_SKEW_MILLIS: u64 = 30_000;
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
                    let client = client.clone();
                    let token = token.clone();
                    let node = self_node.clone();
                    joins.spawn(async move {
                        let result = contact_peer(&client, &endpoint, &token, node).await;
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
                            if let Err(error) = replicate_peer(&state, &client, &endpoint, &token).await {
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
    client: &reqwest::Client,
    endpoint: &str,
    token: &str,
    node: NodeRecord,
) -> Result<ClusterJoinResponse> {
    let mut url = reqwest::Url::parse(endpoint)
        .map_err(|error| LiveError::Config(format!("invalid cluster endpoint: {error}")))?;
    url.set_path(JOIN_PATH);
    let body = serde_json::to_vec(&ClusterJoinRequest { node })?;
    let timestamp = now_millis().to_string();
    let signature = sign(token, &timestamp, &body);
    let response = client
        .post(url)
        .header(TIMESTAMP_HEADER, &timestamp)
        .header(SIGNATURE_HEADER, signature)
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

pub fn verify(token: &str, timestamp: &str, signature: &str, body: &[u8]) -> Result<()> {
    let timestamp_millis = timestamp
        .parse::<u64>()
        .map_err(|_| LiveError::Authentication("invalid cluster join timestamp".to_owned()))?;
    if now_millis().abs_diff(timestamp_millis) > MAX_CLOCK_SKEW_MILLIS {
        return Err(LiveError::Authentication(
            "cluster join timestamp is outside the allowed clock window".to_owned(),
        ));
    }
    let expected = sign(token, timestamp, body);
    if !constant_time_eq(expected.as_bytes(), signature.as_bytes()) {
        return Err(LiveError::Authentication(
            "invalid cluster join signature".to_owned(),
        ));
    }
    Ok(())
}

fn sign(token: &str, timestamp: &str, body: &[u8]) -> String {
    let key = blake3::derive_key(SIGNING_CONTEXT, token.as_bytes());
    let mut hasher = blake3::Hasher::new_keyed(&key);
    hasher.update(timestamp.as_bytes());
    hasher.update(b"\n");
    hasher.update(body);
    hasher.finalize().to_hex().to_string()
}

async fn replicate_peer(
    state: &AppState,
    client: &reqwest::Client,
    endpoint: &str,
    token: &str,
) -> Result<()> {
    let max_objects = state.config().cluster.replication_max_objects_per_round;
    let max_bytes = state.config().cluster.replication_max_object_bytes;
    let mut cursor = None;
    let mut transferred = 0_usize;
    loop {
        let mut inventory_url = cluster_url(endpoint, OBJECTS_PATH)?;
        let remaining = max_objects.saturating_sub(transferred);
        if remaining == 0 {
            break;
        }
        let limit = remaining.min(ObjectQuery::MAX_LIMIT as usize);
        {
            let mut query = inventory_url.query_pairs_mut();
            query.append_pair("limit", &limit.to_string());
            if let Some(cursor) = cursor.as_deref() {
                query.append_pair("cursor", cursor);
            }
        }
        let response = signed_get(client, inventory_url, token).await?;
        let inventory: ObjectPage = response.json().await.map_err(|error| {
            LiveError::Protocol(format!(
                "decode cluster object inventory from {endpoint}: {error}"
            ))
        })?;
        if inventory.truncated {
            return Err(LiveError::Capability(format!(
                "cluster object inventory from {endpoint} was truncated by its provider"
            )));
        }
        let next_cursor = inventory.next_cursor.clone();
        for metadata in inventory.objects {
            transferred = transferred.saturating_add(1);
            if metadata.size > max_bytes {
                tracing::warn!(object = %metadata.id, size = metadata.size, max_bytes, "skipping oversized cluster object");
                continue;
            }
            let id = metadata.id.clone();
            let registry = state.registry().clone();
            let present = tokio::task::spawn_blocking(move || registry.get_object(&id).is_ok())
                .await
                .map_err(|error| {
                    LiveError::Conflict(format!("join cluster object lookup: {error}"))
                })?;
            if present {
                continue;
            }
            let url = cluster_url(endpoint, &format!("{OBJECTS_PATH}/{}", metadata.id))?;
            let response = signed_get(client, url, token).await?;
            if !response.status().is_success() {
                return Err(LiveError::Transport(format!(
                    "fetch cluster object {} from {endpoint}: HTTP {}",
                    metadata.id,
                    response.status()
                )));
            }
            let media_type = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("application/octet-stream")
                .to_owned();
            let filename = response
                .headers()
                .get("x-hologram-object-filename")
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned);
            let mut bytes = Vec::new();
            let mut response = response;
            while let Some(chunk) = response.chunk().await.map_err(|error| {
                LiveError::Transport(format!("read cluster object {}: {error}", metadata.id))
            })? {
                if (bytes.len() as u64).saturating_add(chunk.len() as u64) > max_bytes {
                    return Err(LiveError::Capability(format!(
                        "cluster object {} exceeds {max_bytes} byte transfer bound",
                        metadata.id
                    )));
                }
                bytes.extend_from_slice(&chunk);
            }
            let registry = state.registry().clone();
            let kind = metadata.kind;
            let expected = metadata.id;
            let stored = tokio::task::spawn_blocking(move || {
                registry.put_object(kind, media_type, filename, &bytes)
            })
            .await
            .map_err(|error| {
                LiveError::Conflict(format!("join cluster object import: {error}"))
            })??;
            if stored.id != expected {
                return Err(LiveError::Protocol(format!(
                    "cluster object digest mismatch: expected {expected}, got {}",
                    stored.id
                )));
            }
        }
        match next_cursor {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    Ok(())
}

fn cluster_url(endpoint: &str, path: &str) -> Result<reqwest::Url> {
    let mut url = reqwest::Url::parse(endpoint)
        .map_err(|error| LiveError::Config(format!("invalid cluster endpoint: {error}")))?;
    url.set_path(path);
    Ok(url)
}

async fn signed_get(
    client: &reqwest::Client,
    url: reqwest::Url,
    token: &str,
) -> Result<reqwest::Response> {
    let timestamp = now_millis().to_string();
    client
        .get(url)
        .header(TIMESTAMP_HEADER, &timestamp)
        .header(SIGNATURE_HEADER, sign(token, &timestamp, &[]))
        .send()
        .await
        .map_err(|error| LiveError::Transport(format!("send cluster replication request: {error}")))
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

    #[test]
    fn signed_join_proof_covers_timestamp_and_body() {
        let timestamp = now_millis().to_string();
        let body = br#"{"node":{"node_id":"one"}}"#;
        let signature = sign("cluster secret", &timestamp, body);

        verify("cluster secret", &timestamp, &signature, body).expect("valid proof");
        assert!(verify("cluster secret", &timestamp, &signature, b"changed").is_err());
        assert!(verify("other secret", &timestamp, &signature, body).is_err());
    }

    #[test]
    fn stale_join_proof_is_rejected() {
        let timestamp = now_millis()
            .saturating_sub(MAX_CLOCK_SKEW_MILLIS + 1)
            .to_string();
        let body = b"{}";
        let signature = sign("cluster secret", &timestamp, body);
        assert!(verify("cluster secret", &timestamp, &signature, body).is_err());
    }

    #[test]
    fn peer_object_paths_do_not_replace_the_advertised_origin() {
        let url = cluster_url("https://node.example:11435", OBJECTS_PATH).expect("inventory URL");
        assert_eq!(
            url.as_str(),
            "https://node.example:11435/api/v1/cluster/objects"
        );
        let object = cluster_url(
            "https://node.example:11435",
            "/api/v1/cluster/objects/blake3:abc",
        )
        .expect("object URL");
        assert_eq!(object.path(), "/api/v1/cluster/objects/blake3:abc");
    }
}
