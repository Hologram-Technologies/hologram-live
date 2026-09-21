//! Authenticated seed-based server membership for Hologram nodes.
//!
//! Registry content replication remains the responsibility of Kappa Registry.
//! This layer discovers live Hologram frontends and deliberately does not retry
//! an in-flight OCI mutation against another authority.

use crate::app::AppState;
use crate::config::validate_cluster_endpoint;
use crate::error::{LiveError, Result};
use crate::protocol::{ClusterJoinRequest, ClusterJoinResponse, NodeRecord};
use crate::util::{constant_time_eq, now_millis};
use std::collections::BTreeSet;
use std::time::Duration;
use tokio::task::{JoinHandle, JoinSet};

pub const JOIN_PATH: &str = "/api/v1/cluster/join";
pub const TIMESTAMP_HEADER: &str = "x-hologram-cluster-timestamp";
pub const SIGNATURE_HEADER: &str = "x-hologram-cluster-signature";
const SIGNING_CONTEXT: &str = "dev.hologram.live.cluster-join.v1";
const MAX_CLOCK_SKEW_MILLIS: u64 = 30_000;
pub const MAX_JOIN_BYTES: usize = 1024 * 1024;

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
    let Some(token) = state.config().cluster_token() else {
        tracing::error!("cluster token disappeared after configuration validation");
        return;
    };
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
}
