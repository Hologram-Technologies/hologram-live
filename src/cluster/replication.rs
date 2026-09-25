//! Immutable object reconciliation against a cluster peer.

use super::network::{ClusterResponse, NetworkRegistry};
use super::{proof, recipient_for, signed_request, OBJECTS_PATH};
use crate::app::AppState;
use crate::error::{LiveError, Result};
use crate::protocol::{ObjectPage, ObjectQuery};

/// Per-object outcomes for one replication round. Data-error object
/// failures are counted and logged; the round itself ends early only on an
/// inventory-level failure or a transport/authorization failure fetching one
/// object (see [`ends_replication_round`]), since either means the peer
/// itself, not just one object, is the problem.
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
}

/// Transport and authentication/authorization failures mean the *peer* is
/// the problem — unreachable, timing out, or has revoked our access — not
/// the one object being fetched when the failure surfaced. Continuing to
/// iterate the rest of the inventory against a peer in that state would
/// retry the same doomed request object after object (worst case:
/// `replication_max_objects_per_round` sequential `request_timeout_secs`
/// timeouts against one dead peer), and it happens synchronously inside the
/// per-round join loop in `cluster::run`, delaying bookkeeping for every
/// other peer contacted that round.
///
/// A data error — an oversize transfer, a digest mismatch, a local store
/// failure, or a lookup/import join failure — is specific to the one object
/// it was raised for and must not stop objects that would otherwise
/// succeed.
fn ends_replication_round(error: &LiveError) -> bool {
    matches!(
        error,
        LiveError::Transport(_) | LiveError::Authentication(_) | LiveError::Authorization(_)
    )
}

pub(super) async fn replicate_peer(
    state: &AppState,
    networks: &NetworkRegistry,
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
        let response =
            signed_get(state, networks, endpoint, &inventory_url, token, &recipient).await?;
        let inventory: ObjectPage = serde_json::from_slice(&response.body).map_err(|error| {
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
            // A data-error object failure (oversize transfer, digest
            // mismatch, a local store failure) must not abort the rest of
            // this peer's inventory: it is recorded and the round
            // continues. A transport or authentication/authorization
            // failure fetching this object — like the inventory request and
            // its decode above — ends the round instead, via
            // `ends_replication_round` below: it means the peer itself is
            // unreachable or has revoked our access, not that this one
            // object is bad.
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
                let response =
                    signed_get(state, networks, endpoint, &url, token, &recipient).await?;
                if !response.is_success() {
                    return Err(fetch_failure(
                        response.status,
                        format!(
                            "fetch cluster object {} from {endpoint}: HTTP {}",
                            metadata.id, response.status
                        ),
                    ));
                }
                let media_type = response
                    .header(reqwest::header::CONTENT_TYPE.as_str())
                    .unwrap_or("application/octet-stream")
                    .to_owned();
                let filename = response
                    .header("x-hologram-object-filename")
                    .map(str::to_owned);
                // Still bounded here, and still before the bytes are hashed or
                // stored. A `ClusterResponse` carries the body it read, so what
                // was a per-chunk bound is now one bound on the whole body;
                // `metadata.size > max_bytes` above already skipped whatever
                // the peer admitted was oversize, and this catches a peer whose
                // bytes do not match its own inventory.
                if response.body.len() as u64 > max_bytes {
                    return Err(LiveError::Capability(format!(
                        "cluster object {} exceeds {max_bytes} byte transfer bound",
                        metadata.id
                    )));
                }
                let bytes = response.body;
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
                Err(error) if ends_replication_round(&error) => {
                    tracing::debug!(
                        peer = %endpoint,
                        stored = outcome.stored,
                        failed = outcome.failed,
                        %error,
                        "cluster replication round ended early: the peer, not one object, is the problem"
                    );
                    return Err(error);
                }
                Err(error) => outcome.record_object_failure(&object_id, &error),
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

/// The status classification [`ends_replication_round`] depends on, kept on
/// this side of the network seam so it survives the transport becoming
/// pluggable: 401 is this node's standing with the peer, 403 the peer's
/// decision about this node, and every other failure status is the peer being
/// unusable this round. All three end the round; none is a fact about the one
/// object being fetched. Matched on the numeric status rather than
/// `reqwest::StatusCode` because a [`ClusterResponse`] comes from whichever
/// network answered.
fn fetch_failure(status: u16, message: String) -> LiveError {
    match status {
        401 => LiveError::Authentication(message),
        403 => LiveError::Authorization(message),
        _ => LiveError::Transport(message),
    }
}

pub(super) fn cluster_url(endpoint: &str, path: &str) -> Result<reqwest::Url> {
    let mut url = reqwest::Url::parse(endpoint)
        .map_err(|error| LiveError::Config(format!("invalid cluster endpoint: {error}")))?;
    url.set_path(path);
    Ok(url)
}

/// `url` is built from `endpoint` by [`cluster_url`], so the path and query the
/// proof binds are exactly the ones the request carries; `endpoint` is what the
/// registry routes on, and `recipient` is the origin the proof was minted for.
pub(super) async fn signed_get(
    state: &AppState,
    networks: &NetworkRegistry,
    endpoint: &str,
    url: &reqwest::Url,
    token: &str,
    recipient: &str,
) -> Result<ClusterResponse> {
    proof::reject_separators("GET", url.path(), url.query(), recipient)?;
    let request_proof = proof::sign_request(
        state.identity(),
        recipient,
        "GET",
        url.path(),
        url.query(),
        &[],
    );
    networks
        .send(
            endpoint,
            signed_request(
                "GET",
                url,
                Vec::new(),
                recipient,
                &request_proof,
                token,
                None,
            ),
        )
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    // A data error never ends a round: `RoundOutcome` just counts it. (Note
    // the fix-round-1 correction below: an actual `LiveError::Transport`
    // reaching `replicate_peer`'s per-object match arm ends the round via
    // `ends_replication_round`, rather than being recorded here — this test
    // exercises `RoundOutcome`'s own counters directly, independent of that
    // classification, using `Transport` only as a stand-in payload.)
    #[test]
    fn an_object_failure_does_not_end_the_round() {
        let mut outcome = RoundOutcome::default();
        outcome.record_object_failure("blake3:aa", &LiveError::Capability("too big".to_owned()));
        outcome.record_object_failure(
            "blake3:bb",
            &LiveError::Protocol("digest mismatch".to_owned()),
        );
        outcome.record_object_stored();
        assert_eq!(outcome.failed, 2);
        assert_eq!(outcome.stored, 1);
    }

    // Fix round 1, finding 2: transport and authentication/authorization
    // failures mean the peer itself is the problem and must end the round;
    // data errors are specific to one object and must not.
    #[test]
    fn transport_and_auth_failures_end_the_round_but_data_errors_do_not() {
        assert!(ends_replication_round(&LiveError::Transport(
            "connection refused".to_owned()
        )));
        assert!(ends_replication_round(&LiveError::Authentication(
            "HTTP 401".to_owned()
        )));
        assert!(ends_replication_round(&LiveError::Authorization(
            "HTTP 403".to_owned()
        )));
        assert!(!ends_replication_round(&LiveError::Capability(
            "oversize".to_owned()
        )));
        assert!(!ends_replication_round(&LiveError::Protocol(
            "digest mismatch".to_owned()
        )));
        assert!(!ends_replication_round(&LiveError::Conflict(
            "store failure".to_owned()
        )));
    }

    // Task 10: the transport is now a trait, so an object fetch classifies a
    // numeric status rather than a `reqwest::StatusCode`. 401 and 403 must
    // still reach the variants `ends_replication_round` ends a round on, and
    // for the reasons it ends it on them.
    #[test]
    fn an_unauthorized_or_forbidden_object_fetch_still_classifies_as_such() {
        assert!(matches!(
            fetch_failure(401, "HTTP 401".to_owned()),
            LiveError::Authentication(_)
        ));
        assert!(matches!(
            fetch_failure(403, "HTTP 403".to_owned()),
            LiveError::Authorization(_)
        ));
        assert!(matches!(
            fetch_failure(404, "HTTP 404".to_owned()),
            LiveError::Transport(_)
        ));
        assert!(matches!(
            fetch_failure(503, "HTTP 503".to_owned()),
            LiveError::Transport(_)
        ));
        for status in [401_u16, 403, 404, 503] {
            assert!(
                ends_replication_round(&fetch_failure(status, format!("HTTP {status}"))),
                "HTTP {status} must end the round"
            );
        }
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

    // Fix round 1, finding 4: nothing previously drove `replicate_peer`
    // itself through a mixed inventory — only `RoundOutcome`'s counters were
    // exercised in isolation, and `authenticated_peers_replicate_an_
    // immutable_object` (tests/cluster_e2e.rs) is happy-path only. This
    // stands up a real `AppState` (`replicate_peer`'s actual dependency —
    // there is no lighter seam; see `ClusterAuthority` in
    // `src/modules/control_plane.rs` for why one exists on the receiving
    // side but not here) against a **stub peer**: a plain axum router bound
    // to an ephemeral port that serves a fixed two-object inventory and
    // ignores every header `signed_get` sends (the client never asks the
    // peer to verify anything). This is the same in-process stub pattern
    // already used for model backends in `inference::ollama`'s tests, kept
    // local here rather than reusing `tests/cluster_e2e.rs`'s
    // subprocess-per-node harness, which cannot isolate one peer's
    // response.
    #[tokio::test]
    async fn a_digest_mismatch_does_not_block_a_later_object_in_the_same_round() {
        use axum::extract::Path;
        use axum::http::StatusCode;
        use axum::response::IntoResponse;
        use axum::routing::get;
        use axum::{Json, Router};
        use std::collections::HashMap;
        use std::sync::Arc;

        crate::util::install_crypto_provider();
        let bytes_a = b"first object, advertised under a digest that does not match".to_vec();
        let bytes_b = b"second object, correctly advertised and must still replicate".to_vec();
        // Deliberately wrong: a real peer would never advertise a digest
        // that does not match its own bytes, but this is exactly the
        // failure `replicate_peer` must survive without ending the round.
        let wrong_id_a = format!("blake3:{}", "0".repeat(64));
        let real_id_b = format!("blake3:{}", blake3::hash(&bytes_b).to_hex());

        let mut objects_by_id = HashMap::new();
        objects_by_id.insert(wrong_id_a.clone(), bytes_a.clone());
        objects_by_id.insert(real_id_b.clone(), bytes_b.clone());
        let objects_by_id = Arc::new(objects_by_id);

        let page = ObjectPage {
            objects: vec![
                crate::protocol::ObjectMetadata {
                    id: wrong_id_a.clone(),
                    kind: "file".to_owned(),
                    media_type: "text/plain".to_owned(),
                    filename: None,
                    size: bytes_a.len() as u64,
                    created_at_millis: 0,
                },
                crate::protocol::ObjectMetadata {
                    id: real_id_b.clone(),
                    kind: "file".to_owned(),
                    media_type: "text/plain".to_owned(),
                    filename: None,
                    size: bytes_b.len() as u64,
                    created_at_millis: 0,
                },
            ],
            next_cursor: None,
            truncated: false,
        };

        let router = Router::new()
            .route(
                OBJECTS_PATH,
                get(move || {
                    let page = page.clone();
                    async move { Json(page) }
                }),
            )
            .route(
                crate::cluster::OBJECT_PATH,
                get(move |Path(id): Path<String>| {
                    let objects_by_id = objects_by_id.clone();
                    async move {
                        match objects_by_id.get(&id) {
                            Some(bytes) => (StatusCode::OK, bytes.clone()).into_response(),
                            None => StatusCode::NOT_FOUND.into_response(),
                        }
                    }
                }),
            );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind an ephemeral port for the stub peer");
        let address = listener.local_addr().expect("read the bound address");
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        let endpoint = format!("http://{address}");

        let temp = tempfile::tempdir().expect("temp dir for the calling node's state");
        let mut config = crate::config::AppConfig::default();
        config.paths.config_dir = temp.path().join("config");
        config.paths.data_dir = temp.path().join("data");
        config.paths.state_dir = temp.path().join("state");
        config.paths.cache_dir = temp.path().join("cache");
        // `AppState::build` is the only constructor `replicate_peer` can be
        // driven through, and it needs a `TracingHandle`. Uses
        // `observability::init_for_test` rather than `init` directly: a
        // second test elsewhere in this binary that also builds an
        // `AppState` would otherwise race this one for `try_init`'s
        // global subscriber slot and `.expect()` a panic on whichever
        // caller loses — `init_for_test` installs at most once and hands
        // every caller the same handle (see its doc comment).
        let tracing = crate::observability::init_for_test(&config.tracing, &config.telemetry)
            .expect("init the test tracing subscriber");
        let state = AppState::build(config, tracing)
            .await
            .expect("build a real AppState backed by a temp dir");
        let networks = NetworkRegistry::new(vec![Arc::new(
            crate::cluster::network::HttpNetwork::new(reqwest::Client::new()),
        )]);

        replicate_peer(
            &state,
            &networks,
            &endpoint,
            "a token the stub peer never checks",
        )
        .await
        .expect("a digest mismatch on one object must not end the round");

        assert!(
            state.registry().get_object(&wrong_id_a).is_err(),
            "the mismatched object must never be stored under its advertised id"
        );
        let stored_b = state
            .registry()
            .get_object(&real_id_b)
            .expect("the later, correctly advertised object must still replicate");
        assert_eq!(stored_b.bytes, bytes_b);
    }
}
