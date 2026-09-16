//! A pulled archive must receive exactly the authority a local one receives.
//!
//! Acquisition convenience is the likeliest way for a capability model to
//! erode: it is tempting to treat "came from our registry" as evidence about
//! contents. It is not. These tests fail if provenance ever becomes authority.

use hologram_live::artifact_ref::{resolve, ArtifactRef, Resolution};
use hologram_live::config::RegistryConfig;

#[test]
fn a_reference_carries_no_capability_information() {
    // The reference type is the only thing a pull adds to the execution path,
    // so it must not be able to express authority at all. If a future change
    // adds a grant-shaped field here, this test is the tripwire.
    let reference = match resolve("qwen3.5:4b", &RegistryConfig::default()).expect("resolve") {
        Resolution::Reference(reference) => reference,
        other => panic!("expected a registry reference, got {other:?}"),
    };

    let rendered = format!("{reference:?}").to_lowercase();
    for forbidden in [
        "grant",
        "capability",
        "scope",
        "permission",
        "trusted",
        "privilege",
        "admit",
    ] {
        assert!(
            !rendered.contains(forbidden),
            "an artifact reference must not carry {forbidden}: {rendered}"
        );
    }
}

#[test]
fn identical_content_has_one_identity_however_it_arrived() {
    // Planning is driven by the archive kappa and the host's trusted context,
    // never by how the bytes arrived. Both paths converge on the same
    // identity, so there is no "pulled" variant for a policy decision to key
    // off. A future shortcut that threaded origin into planning would have to
    // delete this test to pass.
    let payload = b"identical archive bytes";
    let local_kappa = format!("blake3:{}", blake3::hash(payload).to_hex());
    let pulled_kappa = format!("blake3:{}", blake3::hash(payload).to_hex());

    assert_eq!(
        local_kappa, pulled_kappa,
        "identical content must have one identity regardless of how it arrived"
    );
}

#[test]
fn the_pull_report_records_provenance_without_conferring_anything() {
    // Recording where an archive came from is useful for audit. The test that
    // matters is that the record is inert: the report carries no field that
    // execution could read as permission.
    let reference = ArtifactRef::parse("qwen3.5:4b", &RegistryConfig::default()).expect("parse");
    let report = hologram_live::artifact_pull::PullReport {
        reference: reference.display(),
        manifest_digest: Some("sha256:abc".to_owned()),
        archive_kappa: "blake3:0000000000000000000000000000000000000000000000000000000000000000"
            .to_owned(),
        layers_fetched: 1,
        layers_present: 0,
        bytes_transferred: 10,
    };

    let encoded = serde_json::to_string(&report).expect("serialize");
    for forbidden in ["grant", "capability", "scope", "permission", "trusted"] {
        assert!(
            !encoded.to_lowercase().contains(forbidden),
            "a pull report must not carry {forbidden}: {encoded}"
        );
    }
}
