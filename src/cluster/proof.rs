//! The authenticated proof carried by every cluster request.
//!
//! The signature binds the method, path, query, recipient, timestamp, and body
//! digest, so a captured proof cannot be replayed against a different route or
//! a different peer.
//!
//! The preimage is newline-separated, so every string field it binds must be
//! free of a line separator. [`reject_separators`] enforces that for `method`,
//! `path`, `query` and `recipient`, and **both** sides call it — the signer in
//! `src/cluster/{mod,replication}.rs` before minting a proof, the verifier in
//! `src/modules/control_plane.rs` before checking one. `timestamp` needs no
//! guard: [`verify_request`] parses it as a `u64` before it reaches the
//! preimage, so it is digits or nothing, and the body is bound only as a
//! fixed-width blake3 digest.

use crate::cluster::identity::{verify_signature, NodeIdentity};
use crate::error::{LiveError, Result};
use crate::util::now_millis;

pub const NODE_HEADER: &str = "x-hologram-cluster-node";
pub const TIMESTAMP_HEADER: &str = "x-hologram-cluster-timestamp";
pub const SIGNATURE_HEADER: &str = "x-hologram-cluster-signature";
pub const TICKET_HEADER: &str = "x-hologram-cluster-ticket";
/// The recipient the caller signed against, echoed so the receiver can compare
/// it rather than guess which spelling of itself was used. Untrusted on its
/// own: the signature must verify over this exact value, and it must then be
/// shown to name the receiving node.
pub const RECIPIENT_HEADER: &str = "x-hologram-cluster-recipient";
/// The sender's membership epoch (see [`crate::ownership::epoch`]), carried so
/// a receiver could in principle detect that its admitted set differs from the
/// sender's. Not part of the signed preimage: it is informational rather than
/// authenticating.
///
/// Attached on outbound cluster-join requests (`cluster::contact_peer`) but
/// **not currently enforced on receipt** — see the note in that function for
/// why admitted sets converge but not atomically, so an equality check would
/// still see transient mismatches during that window (issue #184 tracks safe
/// enforcement, which needs sender-side refresh-and-retry). The header is
/// still sent so the wire carries this information for a corrected consumer.
pub const EPOCH_HEADER: &str = "x-hologram-cluster-epoch";
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

/// Rejects a preimage field carrying a line separator.
///
/// The preimage separates its fields with `\n`, so a field containing one
/// could in principle shift the field layout a verifier reads. No collision is
/// reachable today — an injected separator yields eight segments where every
/// verifier-side field is separator-free, so it cannot match the seven-segment
/// form — but the guard runs on both the signing and the verifying side and
/// fails closed, because a one-sided guard is exactly how a later edit
/// introduces a real collision.
///
/// `recipient` is not structurally safe despite coming from a validated
/// endpoint: `url::Url::parse` silently *strips* interior tab/CR/LF, so
/// `validate_cluster_endpoint` accepts a configured endpoint containing one
/// while `normalize_endpoint` passes the raw bytes straight through.
///
/// The offending value is never included in the error: only the field name.
pub fn reject_separators(
    method: &str,
    path: &str,
    query: Option<&str>,
    recipient: &str,
) -> Result<()> {
    for (field, value) in [
        ("method", Some(method)),
        ("path", Some(path)),
        ("query", query),
        ("recipient", Some(recipient)),
    ] {
        if value.is_some_and(|value| value.contains('\n') || value.contains('\r')) {
            return Err(LiveError::Authentication(format!(
                "cluster request {field} contains a line separator"
            )));
        }
    }
    Ok(())
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

/// Whether `claimed_millis` falls within `MAX_CLOCK_SKEW_MILLIS` of
/// `reference_millis`, inclusive on both ends.
///
/// Pulled out as a pure function (rather than inlining `now_millis()`
/// directly into the comparison) so the exact `>` vs `>=` boundary can be
/// pinned deterministically in tests, without racing the wall clock.
/// `abs_diff` makes the window symmetric: a clock running ahead is refused
/// exactly as firmly as one running behind.
fn within_clock_window(reference_millis: u64, claimed_millis: u64) -> bool {
    reference_millis.abs_diff(claimed_millis) <= MAX_CLOCK_SKEW_MILLIS
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
    if !within_clock_window(now_millis(), timestamp_millis) {
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

        verify_request(
            &proof,
            recipient,
            "GET",
            "/api/v1/cluster/objects",
            Some("limit=10"),
            b"",
        )
        .expect("the proof it signed");

        // Every bound field, altered one at a time.
        assert!(
            verify_request(
                &proof,
                recipient,
                "POST",
                "/api/v1/cluster/objects",
                Some("limit=10"),
                b""
            )
            .is_err(),
            "method"
        );
        assert!(
            verify_request(
                &proof,
                recipient,
                "GET",
                "/api/v1/cluster/join",
                Some("limit=10"),
                b""
            )
            .is_err(),
            "path"
        );
        assert!(
            verify_request(
                &proof,
                recipient,
                "GET",
                "/api/v1/cluster/objects",
                Some("limit=99"),
                b""
            )
            .is_err(),
            "query"
        );
        assert!(
            verify_request(
                &proof,
                "ed25519:bb",
                "GET",
                "/api/v1/cluster/objects",
                Some("limit=10"),
                b""
            )
            .is_err(),
            "recipient"
        );
        assert!(
            verify_request(
                &proof,
                recipient,
                "GET",
                "/api/v1/cluster/objects",
                Some("limit=10"),
                b"body"
            )
            .is_err(),
            "body"
        );
    }

    // Fix round 2, finding B: the guard covers every string field the preimage
    // binds, not just the request target, and it is the same function on both
    // sides. `recipient` is included because a configured endpoint can carry a
    // separator that `Url::parse` strips but `normalize_endpoint` does not.
    #[test]
    fn every_preimage_field_is_screened_for_a_line_separator() {
        reject_separators("GET", "/p", Some("a=1"), "ed25519:aa").expect("clean fields");
        reject_separators("GET", "/p", None, "https://node.example").expect("no query");

        assert!(
            reject_separators("GE\nT", "/p", None, "ed25519:aa").is_err(),
            "method"
        );
        assert!(
            reject_separators("GET", "/p\nx", None, "ed25519:aa").is_err(),
            "path"
        );
        assert!(
            reject_separators("GET", "/p", Some("a=1\nb=2"), "ed25519:aa").is_err(),
            "query"
        );
        assert!(
            reject_separators("GET", "/p", None, "https://node.example\nx").is_err(),
            "recipient"
        );
        assert!(
            reject_separators("GET", "/p\rx", None, "ed25519:aa").is_err(),
            "carriage return"
        );

        // The message names the field and never echoes the value.
        let error =
            reject_separators("GET", "/secret\npath", None, "ed25519:aa").expect_err("must reject");
        let message = error.to_string();
        assert!(message.contains("path"), "{message}");
        assert!(!message.contains("secret"), "{message}");
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

    // Finding (fix round 1): the bidirectional test above only proves the
    // window rejects *far* offsets in both directions — it never pins the
    // `<=` vs `<` boundary the design depends on. Testing `within_clock_window`
    // directly (rather than through `verify_request`, which reads the real
    // wall clock) makes all four corners exact and deterministic instead of
    // racing `now_millis()`.
    #[test]
    fn the_clock_window_boundary_is_exact_on_both_sides() {
        let reference = 1_700_000_000_000_u64;

        assert!(
            within_clock_window(reference, reference - MAX_CLOCK_SKEW_MILLIS),
            "exactly at the past boundary must be accepted"
        );
        assert!(
            !within_clock_window(reference, reference - MAX_CLOCK_SKEW_MILLIS - 1),
            "one millisecond past the past boundary must be rejected"
        );
        assert!(
            within_clock_window(reference, reference + MAX_CLOCK_SKEW_MILLIS),
            "exactly at the future boundary must be accepted"
        );
        assert!(
            !within_clock_window(reference, reference + MAX_CLOCK_SKEW_MILLIS + 1),
            "one millisecond past the future boundary must be rejected"
        );
    }
}
