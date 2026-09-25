//! Immutable object reconciliation against a cluster peer.

use super::{proof, recipient_for, sign_headers, OBJECTS_PATH};
use crate::app::AppState;
use crate::error::{LiveError, Result};
use crate::protocol::{ObjectPage, ObjectQuery};

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

    // Takes `&self`, though nothing here reads it yet: this is the single
    // policy point a future per-round abort condition would extend, and the
    // call site (`replicate_peer`) already asks the outcome rather than
    // hardcoding `true`, so that extension would need no call-site change.
    #[allow(clippy::unused_self)]
    pub const fn should_continue(&self) -> bool {
        true
    }
}

pub(super) async fn replicate_peer(
    state: &AppState,
    client: &reqwest::Client,
    endpoint: &str,
    token: &str,
) -> Result<()> {
    let recipient = recipient_for(endpoint);
    let max_objects = state.config().cluster.replication_max_objects_per_round;
    let max_bytes = state.config().cluster.replication_max_object_bytes;
    let mut cursor = None;
    let mut transferred = 0_usize;
    let mut outcome = RoundOutcome::default();
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
        let response = signed_get(state, client, inventory_url, token, &recipient).await?;
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
            let object_id = metadata.id.clone();
            // A per-object failure (transport hiccup, oversize transfer, a
            // digest mismatch) must not abort the rest of this peer's
            // inventory: it is recorded and the round continues. Only the
            // inventory request and its decode above are allowed to end the
            // round early, since without an inventory there is nothing left
            // to reconcile.
            let result: Result<bool> = async {
                let id = metadata.id.clone();
                let registry = state.registry().clone();
                let present = tokio::task::spawn_blocking(move || registry.get_object(&id).is_ok())
                    .await
                    .map_err(|error| {
                        LiveError::Conflict(format!("join cluster object lookup: {error}"))
                    })?;
                if present {
                    return Ok(false);
                }
                let url = cluster_url(endpoint, &format!("{OBJECTS_PATH}/{}", metadata.id))?;
                let response = signed_get(state, client, url, token, &recipient).await?;
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
                let kind = metadata.kind.clone();
                let expected = metadata.id.clone();
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
                Ok(true)
            }
            .await;
            match result {
                Ok(true) => outcome.record_object_stored(),
                Ok(false) => {}
                Err(error) => outcome.record_object_failure(&object_id, &error),
            }
            // Always true today: a per-object failure never ends a round.
            // Checked explicitly (rather than inlining `true`) so the round
            // loop's continuation is driven by `RoundOutcome`'s own policy,
            // not by an assumption duplicated at the call site.
            if !outcome.should_continue() {
                break;
            }
        }
        match next_cursor {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    tracing::debug!(
        peer = %endpoint,
        stored = outcome.stored,
        failed = outcome.failed,
        "cluster replication round complete"
    );
    Ok(())
}

pub(super) fn cluster_url(endpoint: &str, path: &str) -> Result<reqwest::Url> {
    let mut url = reqwest::Url::parse(endpoint)
        .map_err(|error| LiveError::Config(format!("invalid cluster endpoint: {error}")))?;
    url.set_path(path);
    Ok(url)
}

pub(super) async fn signed_get(
    state: &AppState,
    client: &reqwest::Client,
    url: reqwest::Url,
    token: &str,
    recipient: &str,
) -> Result<reqwest::Response> {
    proof::reject_separators("GET", url.path(), url.query(), recipient)?;
    let request_proof = proof::sign_request(
        state.identity(),
        recipient,
        "GET",
        url.path(),
        url.query(),
        &[],
    );
    sign_headers(client.get(url), recipient, &request_proof, token)
        .send()
        .await
        .map_err(|error| LiveError::Transport(format!("send cluster replication request: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_object_failure_does_not_end_the_round() {
        let mut outcome = RoundOutcome::default();
        outcome.record_object_failure("blake3:aa", &LiveError::Transport("gone".to_owned()));
        outcome.record_object_failure("blake3:bb", &LiveError::Capability("too big".to_owned()));
        outcome.record_object_stored();
        assert_eq!(outcome.failed, 2);
        assert_eq!(outcome.stored, 1);
        assert!(
            outcome.should_continue(),
            "per-object failures never end a round"
        );
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
