//! Immutable object reconciliation against a cluster peer.

use super::network::{ClusterResponse, NetworkRegistry};
use super::{proof, signed_request, SignedTarget, OBJECTS_PATH};
use crate::app::AppState;
use crate::error::{LiveError, Result};
use crate::protocol::{ObjectPage, ObjectQuery};

/// The inventory GET's response ceiling (issue #189, item 2). Until this
/// bound the inventory was the one unbounded read left on the cluster path:
/// object fetches and join replies carry a ceiling the networks enforce
/// while reading, and this call passed `None`.
///
/// The figure is derived, not invented. A page holds at most
/// `ObjectQuery::MAX_LIMIT` entries (1000). Per entry the bounded fields
/// cost little — the id is at most 71 bytes, a validated filename at most
/// 255, two timestamps of at most 20 digits, JSON overhead around a hundred
/// bytes — while `kind` and `media_type` carry no length validation, so the
/// per-entry budget for them is policy: 4 KiB an entry is far above anything
/// a realistic store produces (a 2 KiB media type is pathological) and far
/// below what used to be buffered without limit. The framing allowance
/// covers the cursor and page structure. A peer whose page legitimately
/// exceeds this fails its round loudly — the same `Capability`
/// classification an oversize object gets — rather than being truncated or
/// read without bound.
const INVENTORY_MAX_RESPONSE_BYTES: u64 = ObjectQuery::MAX_LIMIT as u64 * 4 * 1024 + 64 * 1024;

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
    // The recipient comes from the network that will dial this endpoint: an
    // HTTP origin for `https:` peers, the bare node id for key-addressed ones
    // (see `ClusterNetwork::recipient_for`).
    let recipient = networks
        .recipient_for(endpoint)
        .ok_or_else(|| LiveError::Config(format!("no cluster network can reach {endpoint}")))?;
    let max_objects = state.config().cluster.replication_max_objects_per_round;
    let max_bytes = state.config().cluster.replication_max_object_bytes;
    let mut cursor = None;
    let mut transferred = 0_usize;
    let mut outcome = RoundOutcome::default();
    loop {
        let remaining = max_objects.saturating_sub(transferred);
        if remaining == 0 {
            break;
        }
        let limit = remaining.min(ObjectQuery::MAX_LIMIT as usize);
        // The query is form-encoded directly, without a base URL: a
        // key-addressed endpoint (`iroh:ed25519:…`) has no origin to parse one
        // from, and the proof binds this exact string.
        let query = {
            let mut pairs = url::form_urlencoded::Serializer::new(String::new());
            pairs.append_pair("limit", &limit.to_string());
            if let Some(cursor) = cursor.as_deref() {
                pairs.append_pair("cursor", cursor);
            }
            pairs.finish()
        };
        // The inventory now carries a ceiling like every other cluster read,
        // derived from the page's own bounds — see the constant above.
        let response = signed_get(
            state,
            networks,
            endpoint,
            SignedTarget {
                path: OBJECTS_PATH,
                query: Some(&query),
                recipient: &recipient,
            },
            token,
            Some(INVENTORY_MAX_RESPONSE_BYTES),
        )
        .await?;
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
                let path = format!("{OBJECTS_PATH}/{}", metadata.id);
                // The transfer bound, enforced while the object is read.
                let response = signed_get(
                    state,
                    networks,
                    endpoint,
                    SignedTarget {
                        path: &path,
                        query: None,
                        recipient: &recipient,
                    },
                    token,
                    Some(max_bytes),
                )
                .await?;
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
                // Unreachable over HTTP, which refuses an oversize body
                // mid-read and raises the same `Capability` variant this does,
                // so the round continues either way. Kept as the backstop for a
                // network implementation that ignores `max_response_bytes`.
                // (`metadata.size > max_bytes` above still skips whatever the
                // peer *admits* is oversize before any request is made.)
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

/// The proof binds exactly the `path` and `query` the request carries;
/// `endpoint` is what the registry routes on, and `recipient` is what the
/// network dialling `endpoint` derived for the proof (an HTTP origin, or the
/// bare node id for a key-addressed peer).
///
/// `max_response_bytes` is the caller's ceiling on the answer, enforced by the
/// network while it reads (see [`super::network::ClusterNetwork::send`]).
pub(super) async fn signed_get(
    state: &AppState,
    networks: &NetworkRegistry,
    endpoint: &str,
    target: SignedTarget<'_>,
    token: &str,
    max_response_bytes: Option<u64>,
) -> Result<ClusterResponse> {
    proof::reject_separators("GET", target.path, target.query, target.recipient)?;
    let request_proof = proof::sign_request(
        state.identity(),
        target.recipient,
        "GET",
        target.path,
        target.query,
        &[],
    );
    networks
        .send(
            endpoint,
            signed_request(
                "GET",
                target,
                Vec::new(),
                &request_proof,
                token,
                max_response_bytes,
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
        // The path a peer is asked for is built from the route constant alone,
        // so nothing in the endpoint — an origin's path, or a key-addressed
        // endpoint's opaque `iroh:` form — can steer it.
        assert_eq!(
            format!("{OBJECTS_PATH}/blake3:abc"),
            "/api/v1/cluster/objects/blake3:abc"
        );
        // And the query the proof binds is form-encoded exactly once.
        let mut pairs = url::form_urlencoded::Serializer::new(String::new());
        pairs.append_pair("limit", "10");
        pairs.append_pair("cursor", "blake3:ab/c?d e");
        assert_eq!(pairs.finish(), "limit=10&cursor=blake3%3Aab%2Fc%3Fd+e");
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

    /// Final review, FIX 6: `replication_max_objects_per_round` had no test at
    /// all — the only replication tests either asserted `RoundOutcome`'s own
    /// counters or drove a two-object inventory that never reached the bound.
    /// A broken cap means one round against a peer with a large store pulls the
    /// whole store, synchronously, inside `cluster::run`'s join loop.
    ///
    /// The stub peer honours `limit` and `cursor`, like the real inventory
    /// route, and records the limits it was asked for. With the bound at two
    /// against a peer offering five objects, the expected behaviour is exact:
    /// one inventory request for two, two objects stored, and no second
    /// request even though the peer offered a cursor to continue from.
    #[tokio::test]
    async fn the_round_stops_at_replication_max_objects_per_round() {
        use axum::extract::{Path, Query};
        use axum::http::StatusCode;
        use axum::response::IntoResponse;
        use axum::routing::get;
        use axum::{Json, Router};
        use std::collections::HashMap;
        use std::sync::{Arc, Mutex};

        const OFFERED: usize = 5;
        const CAP: usize = 2;

        crate::util::install_crypto_provider();

        let bodies: Vec<Vec<u8>> = (0..OFFERED)
            .map(|index| format!("cluster object number {index}").into_bytes())
            .collect();
        let ids: Vec<String> = bodies
            .iter()
            .map(|bytes| format!("blake3:{}", blake3::hash(bytes).to_hex()))
            .collect();
        let by_id: HashMap<String, Vec<u8>> =
            ids.iter().cloned().zip(bodies.iter().cloned()).collect();
        let by_id = Arc::new(by_id);

        let all: Vec<crate::protocol::ObjectMetadata> = ids
            .iter()
            .zip(bodies.iter())
            .map(|(id, bytes)| crate::protocol::ObjectMetadata {
                id: id.clone(),
                kind: "file".to_owned(),
                media_type: "text/plain".to_owned(),
                filename: None,
                size: bytes.len() as u64,
                created_at_millis: 0,
            })
            .collect();
        let all = Arc::new(all);
        // Every `limit` the peer was asked for, so the bound can be observed on
        // the request and not only in its effect.
        let asked: Arc<Mutex<Vec<usize>>> = Arc::new(Mutex::new(Vec::new()));

        let inventory_asked = asked.clone();
        let router = Router::new()
            .route(
                OBJECTS_PATH,
                get(move |Query(query): Query<HashMap<String, String>>| {
                    let all = all.clone();
                    let asked = inventory_asked.clone();
                    async move {
                        let limit: usize = query
                            .get("limit")
                            .and_then(|value| value.parse().ok())
                            .unwrap_or(OFFERED);
                        let start: usize = query
                            .get("cursor")
                            .and_then(|value| value.parse().ok())
                            .unwrap_or(0);
                        if let Ok(mut asked) = asked.lock() {
                            asked.push(limit);
                        }
                        let end = start.saturating_add(limit).min(all.len());
                        Json(ObjectPage {
                            objects: all[start.min(all.len())..end].to_vec(),
                            next_cursor: (end < all.len()).then(|| end.to_string()),
                            truncated: false,
                        })
                    }
                }),
            )
            .route(
                crate::cluster::OBJECT_PATH,
                get(move |Path(id): Path<String>| {
                    let by_id = by_id.clone();
                    async move {
                        match by_id.get(&id) {
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
        config.cluster.replication_max_objects_per_round = CAP;
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
            "a token the stub never checks",
        )
        .await
        .expect("a capped round is a successful round");

        assert_eq!(
            asked.lock().expect("the recorded limits").as_slice(),
            &[CAP],
            "the round must ask for the bound once and then stop, cursor or not"
        );
        for id in ids.iter().take(CAP) {
            state
                .registry()
                .get_object(id)
                .unwrap_or_else(|error| panic!("object {id} within the bound: {error}"));
        }
        for id in ids.iter().skip(CAP) {
            assert!(
                state.registry().get_object(id).is_err(),
                "object {id} is past replication_max_objects_per_round and must not be fetched"
            );
        }
    }

    /// Builds a real `AppState` against a stub peer, the pattern the
    /// digest-mismatch and round-cap tests above already use: the stub serves
    /// whatever inventory it is given and ignores every header.
    async fn state_and_stub(inventory_body: Vec<u8>) -> (AppState, String, tempfile::TempDir) {
        use axum::response::IntoResponse;
        use axum::routing::get;
        use axum::Router;

        crate::util::install_crypto_provider();
        let objects_router = Router::new().route(
            OBJECTS_PATH,
            get(move || {
                let body = inventory_body.clone();
                async move { (axum::http::StatusCode::OK, body).into_response() }
            }),
        );
        // Every object the inventory names is answered with the same small
        // body, which never hashes to the id it was asked for: a per-object
        // digest mismatch that the round records and continues past, keeping
        // these tests about the inventory read rather than the fetch path.
        let router = objects_router.route(
            crate::cluster::OBJECT_PATH,
            get(|| async { (axum::http::StatusCode::OK, b"stub".to_vec()).into_response() }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind an ephemeral port for the stub peer");
        let address = listener.local_addr().expect("read the bound address");
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });

        let temp = tempfile::tempdir().expect("temp dir for the calling node's state");
        let mut config = crate::config::AppConfig::default();
        config.paths.config_dir = temp.path().join("config");
        config.paths.data_dir = temp.path().join("data");
        config.paths.state_dir = temp.path().join("state");
        config.paths.cache_dir = temp.path().join("cache");
        let tracing = crate::observability::init_for_test(&config.tracing, &config.telemetry)
            .expect("init the test tracing subscriber");
        let state = AppState::build(config, tracing)
            .await
            .expect("build a real AppState backed by a temp dir");
        (state, format!("http://{address}"), temp)
    }

    // Issue #189, item 2, the claim the ceiling's derivation makes: a page at
    // the protocol's own limits — `MAX_LIMIT` entries with maximally long
    // validated fields and generous unvalidated ones — fits under the
    // ceiling, so no legitimate store ever trips it. If someone shrinks the
    // constant below what a maximal page needs, this fails first.
    #[tokio::test]
    async fn a_maximal_legitimate_inventory_page_fits_under_the_ceiling() {
        let entry = |index: usize| crate::protocol::ObjectMetadata {
            id: format!("blake3:{index:064x}"),
            kind: "k".repeat(128),
            media_type: "m".repeat(128),
            filename: Some("f".repeat(255)),
            size: u64::MAX,
            created_at_millis: u64::MAX,
        };
        let page = ObjectPage {
            objects: (0..ObjectQuery::MAX_LIMIT as usize).map(entry).collect(),
            next_cursor: Some("c".repeat(71)),
            truncated: false,
        };
        let body = serde_json::to_vec(&page).expect("encode the maximal page");
        assert!(
            body.len() as u64 <= INVENTORY_MAX_RESPONSE_BYTES,
            "a maximal legitimate page ({} bytes) must fit the {} byte ceiling",
            body.len(),
            INVENTORY_MAX_RESPONSE_BYTES
        );

        let (state, endpoint, _temp) = state_and_stub(body).await;
        let networks = NetworkRegistry::new(vec![std::sync::Arc::new(
            crate::cluster::network::HttpNetwork::new(reqwest::Client::new()),
        )]);
        // The page decodes and the round runs: every object id is
        // well-formed, so each is fetched (and 404s against this stub, a
        // per-object failure that does not end the round).
        replicate_peer(
            &state,
            &networks,
            &endpoint,
            "a token the stub never checks",
        )
        .await
        .expect("a maximal legitimate page must not trip the ceiling");
    }

    // And the other half: a body past the ceiling is refused while it is
    // read, ending the round with `Capability` — the classification the
    // per-object path relies on for oversize answers — never buffered whole.
    #[tokio::test]
    async fn an_inventory_past_the_ceiling_is_refused_not_buffered() {
        let body = vec![
            b'x';
            usize::try_from(INVENTORY_MAX_RESPONSE_BYTES)
                .expect("the ceiling fits usize")
                + 1024
        ];
        let (state, endpoint, _temp) = state_and_stub(body).await;
        let networks = NetworkRegistry::new(vec![std::sync::Arc::new(
            crate::cluster::network::HttpNetwork::new(reqwest::Client::new()),
        )]);
        let error = replicate_peer(
            &state,
            &networks,
            &endpoint,
            "a token the stub never checks",
        )
        .await
        .expect_err("an inventory over the ceiling must fail the round");
        assert!(matches!(error, LiveError::Capability(_)), "got {error:?}");
    }
}
