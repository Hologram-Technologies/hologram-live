//! The client SDK mirrors the daemon's wire shapes instead of importing them,
//! so that a consumer does not inherit wasmtime, axum, and tonic for the sake
//! of a few JSON structures.
//!
//! The cost of that choice is drift. These tests serialize the daemon's own
//! types and deserialize them with the SDK's, so a field renamed on one side
//! fails a build here rather than surfacing as a confusing runtime decode
//! error in somebody's application.

use hologram_live::protocol::{ObjectMetadata, ObjectPage, ObjectQuery};

fn sample() -> ObjectMetadata {
    ObjectMetadata {
        id: "blake3:cb9ef1526f722fcaaf5a6e19610d045349627e4ce70ad4420ccc00b6bfc5e959".to_owned(),
        kind: "file".to_owned(),
        media_type: "text/plain".to_owned(),
        filename: Some("notes.txt".to_owned()),
        size: 24,
        created_at_millis: 1_757_894_400_000,
    }
}

#[test]
fn the_sdk_decodes_the_daemon_object_metadata() {
    let encoded = serde_json::to_string(&sample()).expect("serialize");
    let decoded: hologram_client::ObjectMetadata =
        serde_json::from_str(&encoded).expect("the SDK must decode what the daemon emits");

    assert_eq!(decoded.id, sample().id);
    assert_eq!(decoded.kind, "file");
    assert_eq!(decoded.media_type, "text/plain");
    assert_eq!(decoded.filename.as_deref(), Some("notes.txt"));
    assert_eq!(decoded.size, 24);
    assert_eq!(decoded.created_at_millis, 1_757_894_400_000);
}

#[test]
fn the_sdk_decodes_the_daemon_object_page() {
    let page = ObjectPage {
        objects: vec![sample()],
        next_cursor: Some("blake3:aaa".to_owned()),
        truncated: true,
    };
    let encoded = serde_json::to_string(&page).expect("serialize");
    let decoded: hologram_client::ObjectPage =
        serde_json::from_str(&encoded).expect("the SDK must decode what the daemon emits");

    assert_eq!(decoded.objects.len(), 1);
    assert_eq!(decoded.next_cursor.as_deref(), Some("blake3:aaa"));
    assert!(
        decoded.truncated,
        "truncation must survive: a caller paginating to completion depends on it"
    );
}

#[test]
fn an_absent_filename_survives_the_boundary() {
    let mut metadata = sample();
    metadata.filename = None;
    let encoded = serde_json::to_string(&metadata).expect("serialize");
    let decoded: hologram_client::ObjectMetadata = serde_json::from_str(&encoded).expect("decode");

    assert_eq!(decoded.filename, None);
}

#[test]
fn the_sdk_query_field_names_match_the_daemon_query() {
    // The SDK builds a query string by hand, so its keys must match the names
    // the daemon deserializes. Serializing the daemon's own type is the only
    // authority on what those names are.
    let query = ObjectQuery {
        kind: Some("file".to_owned()),
        media_type: Some("text/plain".to_owned()),
        filename_contains: Some("notes".to_owned()),
        min_size: Some(1),
        max_size: Some(2),
        created_after_millis: Some(3),
        created_before_millis: Some(4),
        limit: 5,
        cursor: Some("blake3:aaa".to_owned()),
    };
    let encoded = serde_json::to_value(&query).expect("serialize");
    let object = encoded.as_object().expect("a JSON object");

    for key in [
        "kind",
        "media_type",
        "filename_contains",
        "min_size",
        "max_size",
        "created_after_millis",
        "created_before_millis",
        "limit",
        "cursor",
    ] {
        assert!(
            object.contains_key(key),
            "the SDK sends {key:?}, so the daemon must still accept it: {object:?}"
        );
    }
}
