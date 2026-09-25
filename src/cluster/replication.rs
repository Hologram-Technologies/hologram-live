//! Immutable object reconciliation against a cluster peer.

use crate::app::AppState;
use crate::error::{LiveError, Result};
use crate::protocol::{ObjectPage, ObjectQuery};
use super::{sign, OBJECTS_PATH, SIGNATURE_HEADER, TIMESTAMP_HEADER};
use crate::util::now_millis;

pub(super) async fn replicate_peer(
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

pub(super) fn cluster_url(endpoint: &str, path: &str) -> Result<reqwest::Url> {
    let mut url = reqwest::Url::parse(endpoint)
        .map_err(|error| LiveError::Config(format!("invalid cluster endpoint: {error}")))?;
    url.set_path(path);
    Ok(url)
}

pub(super) async fn signed_get(
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

#[cfg(test)]
mod tests {
    use super::*;

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
