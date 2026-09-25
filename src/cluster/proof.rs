//! The authenticated proof carried by every cluster request.
//!
//! The signature binds the method, path, query, recipient, timestamp, and body
//! digest, so a captured proof cannot be replayed against a different route or
//! a different peer.
//!
//! This task (Task 3 of the distributed-p2p-clustering plan) adds the module
//! without wiring it in: Task 5 (`src/modules/control_plane.rs`) calls
//! `verify_request` and the header constants. Until that call site lands,
//! everything here outside `#[cfg(test)]` is unreachable from the rest of the
//! crate, which `-D warnings` otherwise turns into a build failure. `expect`
//! (rather than `allow`) means the moment Task 5 wires this in, the lint
//! stops firing and this annotation itself becomes a compile error — a
//! reminder to remove it instead of a silently stale allow.
#![expect(dead_code, reason = "wired in by task 5 of this plan")]

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
