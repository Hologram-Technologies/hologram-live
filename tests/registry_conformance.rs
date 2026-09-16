//! One behavioural contract, executed against every provider.
//!
//! The provider seam exists so storage can move without changing routes,
//! operation ids, or clients. That guarantee is only real if the
//! implementations are observably identical, which is what this asserts.
//!
//! The kappa cases run only when `KAPPA_REGISTRY_ENDPOINT` is set, so a
//! developer without a registry still gets the local half.

use hologram_live::config::RegistryConfig;
use hologram_live::protocol::ObjectQuery;
use hologram_live::registry::{KappaRegistryProvider, LocalRegistryProvider, RegistryProvider};
use hologram_live::store::ObjectStore;
use std::sync::Arc;

/// A local provider over a scratch store. Each case gets its own directory so
/// a shared store cannot make one case depend on another's writes.
fn local(label: &str) -> Box<dyn RegistryProvider> {
    let root = std::env::temp_dir().join(format!(
        "hologram-conformance-{label}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let store = Arc::new(ObjectStore::open(&root).expect("open store"));
    Box::new(LocalRegistryProvider::new(store))
}

/// A kappa provider against a throwaway namespace, or `None` when no endpoint
/// is configured.
fn kappa(label: &str) -> Option<Box<dyn RegistryProvider>> {
    let endpoint = std::env::var("KAPPA_REGISTRY_ENDPOINT").ok()?;
    let config = RegistryConfig {
        provider: "kappa".to_owned(),
        endpoint,
        // A namespace per case and per process: upstream namespaces auto-create
        // on write, and reusing one would let a previous run's objects satisfy
        // or break this one's assertions.
        namespace: format!("conformance-{label}-{}", std::process::id()),
        ..RegistryConfig::default()
    };
    Some(Box::new(
        KappaRegistryProvider::new(&config).expect("build kappa provider"),
    ))
}

/// Run one contract against every available provider, naming which failed.
fn for_each_provider(label: &str, contract: fn(&dyn RegistryProvider, &str)) {
    contract(local(label).as_ref(), "local");
    match kappa(label) {
        Some(provider) => contract(provider.as_ref(), "kappa"),
        None => eprintln!("skipping kappa provider: KAPPA_REGISTRY_ENDPOINT is unset"),
    }
}

fn put_get_round_trip(provider: &dyn RegistryProvider, name: &str) {
    let stored = provider
        .put_object(
            "file".to_owned(),
            "text/plain".to_owned(),
            Some("hello.txt".to_owned()),
            b"hello",
        )
        .unwrap_or_else(|error| panic!("{name}: put failed: {error}"));

    assert!(
        stored.id.starts_with("blake3:"),
        "{name}: object identity must be a blake3 kappa, got {}",
        stored.id
    );
    assert_eq!(stored.size, 5, "{name}: size");
    assert_eq!(stored.kind, "file", "{name}: kind");
    assert_eq!(
        stored.filename.as_deref(),
        Some("hello.txt"),
        "{name}: filename"
    );

    let fetched = provider
        .get_object(&stored.id)
        .unwrap_or_else(|error| panic!("{name}: get failed: {error}"));
    assert_eq!(fetched.bytes, b"hello", "{name}: bytes round-trip");
    assert_eq!(fetched.metadata.id, stored.id, "{name}: identity is stable");
    assert_eq!(
        fetched.metadata.media_type, "text/plain",
        "{name}: media type survives"
    );
}

fn put_is_idempotent(provider: &dyn RegistryProvider, name: &str) {
    let first = provider
        .put_object("file".to_owned(), "text/plain".to_owned(), None, b"same")
        .unwrap_or_else(|error| panic!("{name}: first put: {error}"));
    let second = provider
        .put_object("file".to_owned(), "text/plain".to_owned(), None, b"same")
        .unwrap_or_else(|error| panic!("{name}: second put: {error}"));

    assert_eq!(first.id, second.id, "{name}: content addressing is stable");
    assert_eq!(
        first.created_at_millis, second.created_at_millis,
        "{name}: immutable content keeps its creation time"
    );
}

fn missing_objects_are_not_found(provider: &dyn RegistryProvider, name: &str) {
    let absent = "blake3:0000000000000000000000000000000000000000000000000000000000000000";
    let error = provider
        .get_object(absent)
        .unwrap_err_or_else(name, "a missing object must not succeed");
    assert!(
        matches!(error, hologram_live::error::LiveError::NotFound(_)),
        "{name}: expected NotFound, got {error:?}"
    );
}

/// Small helper so a failing case names its provider, which a bare
/// `expect_err` cannot do.
trait UnwrapErrNamed<T, E> {
    fn unwrap_err_or_else(self, provider: &str, message: &str) -> E;
}

impl<T: std::fmt::Debug, E> UnwrapErrNamed<T, E> for std::result::Result<T, E> {
    fn unwrap_err_or_else(self, provider: &str, message: &str) -> E {
        match self {
            Ok(value) => panic!("{provider}: {message}, got Ok({value:?})"),
            Err(error) => error,
        }
    }
}

fn rename_preserves_identity(provider: &dyn RegistryProvider, name: &str) {
    let stored = provider
        .put_object(
            "file".to_owned(),
            "text/plain".to_owned(),
            Some("before.txt".to_owned()),
            b"rename me",
        )
        .unwrap_or_else(|error| panic!("{name}: put: {error}"));

    let renamed = provider
        .rename_file(&stored.id, "after.txt".to_owned())
        .unwrap_or_else(|error| panic!("{name}: rename: {error}"));

    assert_eq!(
        renamed.id, stored.id,
        "{name}: rename must not change identity"
    );
    assert_eq!(
        renamed.filename.as_deref(),
        Some("after.txt"),
        "{name}: new name"
    );
    let fetched = provider
        .get_object(&stored.id)
        .unwrap_or_else(|error| panic!("{name}: get after rename: {error}"));
    assert_eq!(fetched.bytes, b"rename me", "{name}: content is untouched");
    assert_eq!(
        fetched.metadata.filename.as_deref(),
        Some("after.txt"),
        "{name}: the rename is durable"
    );
}

fn search_filters_and_orders_identically(provider: &dyn RegistryProvider, name: &str) {
    for (index, kind) in ["file", "holo", "file"].iter().enumerate() {
        provider
            .put_object(
                (*kind).to_owned(),
                "text/plain".to_owned(),
                Some(format!("item{index}.txt")),
                format!("payload-{index}").as_bytes(),
            )
            .unwrap_or_else(|error| panic!("{name}: seeding put: {error}"));
    }

    let page = provider
        .search(&ObjectQuery {
            kind: Some("file".to_owned()),
            ..ObjectQuery::default()
        })
        .unwrap_or_else(|error| panic!("{name}: search: {error}"));

    assert_eq!(
        page.objects.len(),
        2,
        "{name}: only file-kind objects match"
    );
    assert!(
        page.objects.iter().all(|object| object.kind == "file"),
        "{name}: the kind filter must be exact"
    );
    let ids: Vec<&str> = page.objects.iter().map(|o| o.id.as_str()).collect();
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    assert_eq!(ids, sorted, "{name}: results ascend by id");
}

fn search_pages_without_overlap(provider: &dyn RegistryProvider, name: &str) {
    for index in 0..4u8 {
        provider
            .put_object(
                "file".to_owned(),
                "text/plain".to_owned(),
                None,
                &[index, index],
            )
            .unwrap_or_else(|error| panic!("{name}: seeding put: {error}"));
    }

    let first = provider
        .search(&ObjectQuery {
            limit: 2,
            ..ObjectQuery::default()
        })
        .unwrap_or_else(|error| panic!("{name}: first page: {error}"));
    assert_eq!(first.objects.len(), 2, "{name}: page size is honoured");

    let cursor = first
        .next_cursor
        .clone()
        .unwrap_or_else(|| panic!("{name}: more results exist, so a cursor is required"));

    let second = provider
        .search(&ObjectQuery {
            limit: 2,
            cursor: Some(cursor),
            ..ObjectQuery::default()
        })
        .unwrap_or_else(|error| panic!("{name}: second page: {error}"));

    let first_ids: Vec<&String> = first.objects.iter().map(|o| &o.id).collect();
    for object in &second.objects {
        assert!(
            !first_ids.contains(&&object.id),
            "{name}: pages overlap on {}",
            object.id
        );
    }
}

fn a_foreign_kind_is_excluded_from_the_files_projection(
    provider: &dyn RegistryProvider,
    name: &str,
) {
    provider
        .put_object(
            "holo".to_owned(),
            "application/octet-stream".to_owned(),
            None,
            b"archive bytes",
        )
        .unwrap_or_else(|error| panic!("{name}: put: {error}"));

    let files = provider
        .list_objects(Some("file"))
        .unwrap_or_else(|error| panic!("{name}: list: {error}"));

    assert!(
        files.iter().all(|object| object.kind == "file"),
        "{name}: a kind-filtered listing must not leak other kinds"
    );
}

#[test]
fn providers_round_trip_objects() {
    for_each_provider("roundtrip", put_get_round_trip);
}

#[test]
fn providers_treat_put_as_idempotent() {
    for_each_provider("idempotent", put_is_idempotent);
}

#[test]
fn providers_report_missing_objects_the_same_way() {
    for_each_provider("missing", missing_objects_are_not_found);
}

#[test]
fn providers_rename_without_changing_identity() {
    for_each_provider("rename", rename_preserves_identity);
}

#[test]
fn providers_filter_and_order_search_identically() {
    for_each_provider("search", search_filters_and_orders_identically);
}

#[test]
fn providers_paginate_search_identically() {
    for_each_provider("paginate", search_pages_without_overlap);
}

#[test]
fn providers_exclude_foreign_kinds_from_a_filtered_listing() {
    for_each_provider(
        "kinds",
        a_foreign_kind_is_excluded_from_the_files_projection,
    );
}
