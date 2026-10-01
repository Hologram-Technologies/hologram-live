//! The iroh-blobs staging cache and its mirror of the registry (Phase 2b,
//! #179).
//!
//! One `FsStore` per node serves both directions of replication between
//! key-addressed peers: outbound, a reconcile pass imports the registry's
//! objects into the store by reference and pins each under a named tag, so
//! the provider side of `iroh-blobs` can serve them; inbound, replication
//! fetches verified ranges into the same store and imports the completed
//! object into the registry, so the node can serve what it just replicated
//! without a second transfer.
//!
//! The store is a cache, never an authority. The named-tag reconcile is what
//! keeps it that way: whatever the registry holds is tagged and served,
//! whatever it no longer holds has its tag deleted and becomes collectable,
//! and nothing is ever served from it that the registry does not hold. The
//! registry decides what an object is, exactly as ADR 021 requires.

use crate::app::AppState;
use crate::error::{LiveError, Result};
use crate::protocol::ObjectMetadata;
use iroh_blobs::api::blobs::{AddPathOptions, ImportMode};
use iroh_blobs::store::fs::FsStore;
use iroh_blobs::{BlobFormat, Hash};
use std::collections::BTreeSet;
use std::sync::Arc;
use tokio_stream::StreamExt;

/// What a network's blob channel can do, in the vocabulary replication
/// speaks: object ids and peer addresses. The iroh implementation translates
/// both (address → public key, object id → blob hash) at its own boundary,
/// so this surface carries no transport types and `replicate_peer` never
/// branches on what is underneath. `ClusterNetwork::blob_channel` hands one
/// of these out, or `None` when the network has no second protocol and the
/// per-object GET keeps serving.
#[async_trait::async_trait]
pub(crate) trait BlobChannel: Send + Sync {
    /// Fetches the object from `peer` into the local blob store, resuming
    /// whatever verified ranges an interrupted earlier attempt left behind,
    /// and returns the payload bytes this attempt actually transferred. The
    /// bytes are verified against the hash the id names as they stream, so a
    /// successful return means the local copy is exactly the object's
    /// content.
    async fn fetch(&self, peer: &str, id: &str) -> Result<u64>;

    /// A blocking reader over the verified local copy, for the registry's
    /// streaming put. The store's reader is async underneath and the bridge
    /// drives it on the captured runtime handle, so the reader must be
    /// consumed where blocking is allowed — replication does so inside
    /// `tokio::task::spawn_blocking`, like every other registry call it makes.
    fn reader(&self, id: &str) -> Result<Box<dyn std::io::Read + Send>>;
}

/// Every tag the mirror manages has this prefix followed by the object's
/// 64-character digest, so the reconcile can enumerate exactly its own tags
/// and a store used for nothing else can hold no other kind.
pub(crate) const TAG_PREFIX: &str = "hologram:";

pub(crate) fn tag_name(digest: &str) -> String {
    format!("{TAG_PREFIX}{digest}")
}

/// The object id ↔ blob hash mapping the whole phase rests on: a blob's id
/// is the plain BLAKE3 hash of its bytes, which is exactly what a registry
/// `blake3:` id carries. There is no translation table; there is only the
/// parse, which still refuses anything malformed rather than trusting the
/// peer that advertised it. `Protocol`, because a malformed advertised id is
/// a fact about the one object, not about this node's store.
pub(crate) fn hash_from_object_id(id: &str) -> Result<Hash> {
    let digest = id
        .strip_prefix("blake3:")
        .ok_or_else(|| LiveError::Protocol(format!("unsupported cluster object id {id:?}")))?;
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(LiveError::Protocol(format!(
            "malformed cluster object id {id:?}"
        )));
    }
    // The length check above makes the narrowing infallible; `unhex` is
    // case-insensitive, and the byte string it decodes names the same hash
    // however the peer capitalized it.
    let bytes = super::identity::unhex(digest)
        .ok_or_else(|| LiveError::Protocol(format!("malformed cluster object id {id:?}")))?;
    let bytes: [u8; 32] = <[u8; 32]>::try_from(bytes.as_slice())
        .map_err(|_| LiveError::Protocol(format!("malformed cluster object id {id:?}")))?;
    Ok(Hash::from_bytes(bytes))
}

/// The canonical spelling of the object id a blob hash names. Always
/// lowercase, which is the form `ObjectStore::put` assigns.
pub(crate) fn object_id_from_hash(hash: &Hash) -> String {
    format!("blake3:{}", hash.to_hex())
}

/// One object of a replication round against a peer whose network offers a
/// blob channel: fetch the verified bytes into the local store, then import
/// them into the registry through the streaming put, so the object never
/// sits whole in memory on this side either. The kind, media type and
/// filename come from the proof-authenticated inventory — the blobs protocol
/// carries bytes and nothing else — and the digest equality backstop the
/// HTTP path performs applies unchanged: bao verification makes it nearly
/// always redundant, and it stays.
pub(crate) async fn fetch_and_import_object(
    state: &AppState,
    channel: &Arc<dyn BlobChannel>,
    peer: &str,
    metadata: &ObjectMetadata,
    max_bytes: u64,
) -> Result<()> {
    channel.fetch(peer, &metadata.id).await?;
    let reader = channel.reader(&metadata.id)?;
    let local = state.local_registry().cloned();
    let registry = state.registry().clone();
    let kind = metadata.kind.clone();
    let media_type = metadata.media_type.clone();
    let filename = metadata.filename.clone();
    let expected = metadata.id.clone();
    let expected_in_closure = expected.clone();
    let stored = tokio::task::spawn_blocking(move || {
        if let Some(local) = local {
            local.put_object_reader(kind, media_type, filename, reader)
        } else {
            // The kappa provider's put crosses the network to the registry
            // service regardless, so it keeps the whole-bytes path — read
            // with the same transfer ceiling the HTTP path enforces, so a
            // peer that understated the size cannot turn the fallback into
            // the buffering defect this phase removes.
            use std::io::Read as _;
            let mut bytes = Vec::new();
            let mut limited = reader.take(max_bytes.saturating_add(1));
            limited.read_to_end(&mut bytes).map_err(LiveError::from)?;
            if bytes.len() as u64 > max_bytes {
                return Err(LiveError::Capability(format!(
                    "cluster object {expected_in_closure} exceeds {max_bytes} byte transfer bound"
                )));
            }
            registry.put_object(kind, media_type, filename, &bytes)
        }
    })
    .await
    .map_err(|error| LiveError::Conflict(format!("join cluster object import: {error}")))??;
    if stored.id != expected {
        return Err(LiveError::Protocol(format!(
            "cluster object digest mismatch: expected {expected}, got {}",
            stored.id
        )));
    }
    Ok(())
}

/// What one mirror pass did to the tag set. Exact, because the test of the
/// reconcile asserts on these counts rather than on the store's internals.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct ReconcileOutcome {
    /// Objects the registry holds that gained a tag this pass.
    pub imported: usize,
    /// Objects already tagged from an earlier pass.
    pub kept: usize,
    /// Tags deleted because the registry no longer holds the object.
    pub removed: usize,
    /// Objects that could not be imported this pass (logged, retried next).
    pub failed: usize,
}

/// Diffs the registry's inventory against the store's `hologram:` tags and
/// makes the tag set exactly the inventory: new objects are imported from
/// the registry's content-addressed blob files — `TryReference`, so a
/// same-filesystem import links rather than copies, which is safe because
/// those files are immutable by construction — and tags whose digest the
/// registry no longer holds are deleted so the collector can reclaim the
/// bytes.
///
/// Runs on the replication cadence from `cluster::run`; first start imports
/// the existing inventory, steady state is a diff. A failure on one object
/// is logged and left for the next pass, because a store that lags the
/// registry is merely a colder cache, while a pass that stops early leaves
/// every later object unmirrored too.
pub(crate) async fn reconcile(state: &AppState, blobs: &FsStore) -> Result<ReconcileOutcome> {
    if state.local_registry().is_none() {
        // Only the local provider's objects exist as files this node can
        // import by reference. A kappa provider's registry holds its bytes
        // remotely, so there is nothing local to mirror; the store still
        // serves whatever inbound fetches landed in it.
        return Ok(ReconcileOutcome::default());
    }
    let registry = state.registry().clone();
    let inventory = tokio::task::spawn_blocking(move || registry.list_objects(None))
        .await
        .map_err(|error| LiveError::Conflict(format!("join blob mirror inventory: {error}")))??;
    let mut expected = BTreeSet::new();
    for metadata in &inventory {
        // Malformed ids cannot come out of `put`, so one here means
        // hand-edited state; skip it rather than mirror a guess at it.
        match metadata.id.strip_prefix("blake3:") {
            Some(digest)
                if digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit()) =>
            {
                expected.insert(digest.to_owned());
            }
            _ => {
                tracing::warn!(object = %metadata.id, "blob mirror skips an object id it cannot name");
            }
        }
    }

    let mut current: BTreeSet<String> = BTreeSet::new();
    let mut listed = blobs
        .tags()
        .list_prefix(TAG_PREFIX)
        .await
        .map_err(|error| LiveError::Io(format!("list blob mirror tags: {error}")))?;
    while let Some(info) = listed.next().await {
        let info =
            info.map_err(|error| LiveError::Io(format!("list blob mirror tags: {error}")))?;
        current.insert(String::from_utf8_lossy(info.name.as_ref()).into_owned());
    }

    let mut outcome = ReconcileOutcome::default();
    for digest in &expected {
        let tag = tag_name(digest);
        if current.contains(&tag) {
            outcome.kept = outcome.kept.saturating_add(1);
            continue;
        }
        let path = state.store().blob_path(digest);
        if !path.exists() {
            outcome.failed = outcome.failed.saturating_add(1);
            tracing::warn!(object = %digest, path = %path.display(), "blob mirror found metadata without a blob file");
            continue;
        }
        let imported = blobs
            .blobs()
            .add_path_with_opts(AddPathOptions {
                path: path.clone(),
                format: BlobFormat::Raw,
                mode: ImportMode::TryReference,
            })
            .with_named_tag(&tag)
            .await;
        match imported {
            Ok(added) if object_id_from_hash(&added.hash) == format!("blake3:{digest}") => {
                outcome.imported = outcome.imported.saturating_add(1);
            }
            Ok(added) => {
                // The file did not hash to its own address — the on-disk
                // corruption the registry's read path refuses to serve. The
                // tag names the address rather than the bytes, so it must
                // not stay and pin the wrong content.
                outcome.failed = outcome.failed.saturating_add(1);
                let actual = added.hash.to_hex();
                let _ = blobs.tags().delete(&tag).await;
                tracing::warn!(object = %digest, %actual, "blob mirror imported bytes that do not hash to their address; tag removed");
            }
            Err(error) => {
                outcome.failed = outcome.failed.saturating_add(1);
                tracing::warn!(object = %digest, %error, "blob mirror failed to import an object");
            }
        }
    }

    let expected_tags: BTreeSet<String> = expected.iter().map(|digest| tag_name(digest)).collect();
    for tag in current.difference(&expected_tags) {
        match blobs.tags().delete(tag).await {
            Ok(_) => outcome.removed = outcome.removed.saturating_add(1),
            // A tag that will not delete keeps its bytes pinned; log it and
            // let the next pass try again rather than fail the whole pass.
            Err(error) => {
                tracing::warn!(%tag, %error, "blob mirror failed to delete a stale tag");
            }
        }
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest_of(id: &str) -> &str {
        id.strip_prefix("blake3:").expect("a blake3 object id")
    }

    async fn tag_exists(blobs: &FsStore, id: &str) -> bool {
        blobs
            .tags()
            .get(tag_name(digest_of(id)))
            .await
            .expect("read a tag")
            .is_some()
    }

    // The golden vector the phase rests on: bytes added to the iroh-blobs
    // store must hash to exactly the `blake3:<hex>` id `ObjectStore::put`
    // assigns the same bytes. If this ever diverges, the inventory's ids and
    // the blobs protocol's hashes no longer name the same content and every
    // fetch would look for bytes no provider can serve.
    #[tokio::test]
    async fn the_blob_hash_is_the_registry_object_id() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let store = crate::store::ObjectStore::open(directory.path().join("registry"))
            .expect("open the object store");
        let bytes: Vec<u8> = (0..65_537_u32).flat_map(u32::to_le_bytes).collect();
        let metadata = store
            .put("file", "application/octet-stream", None, &bytes)
            .expect("whole-bytes put");

        let blobs = FsStore::load(directory.path().join("cluster-blobs"))
            .await
            .expect("open the blob store");
        let added = blobs
            .blobs()
            .add_bytes(bytes.clone())
            .with_named_tag("golden")
            .await
            .expect("import the same bytes into the blob store");

        assert_eq!(object_id_from_hash(&added.hash), metadata.id);
        assert_eq!(
            metadata.id,
            format!("blake3:{}", blake3::hash(&bytes).to_hex()),
            "and the registry id is the plain blake3 of the bytes"
        );
    }

    #[test]
    fn an_object_id_and_a_blob_hash_round_trip_losslessly() {
        let id = format!("blake3:{}", blake3::hash(b"round trip").to_hex());
        let hash = hash_from_object_id(&id).expect("a well-formed id converts");
        assert_eq!(object_id_from_hash(&hash), id);
        assert_eq!(hash, Hash::new(b"round trip"));
    }

    #[test]
    fn malformed_object_ids_are_refused() {
        assert!(hash_from_object_id("blake3:zz").is_err());
        assert!(hash_from_object_id("blake3:abcd").is_err());
        assert!(hash_from_object_id(
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        )
        .is_err());
        assert!(hash_from_object_id("blake3:").is_err());
        assert!(
            hash_from_object_id(
                "blake3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            )
            .is_ok(),
            "a well-formed id converts"
        );
    }

    async fn cluster_state(directory: &std::path::Path) -> AppState {
        let mut config = crate::config::AppConfig::default();
        config.paths.config_dir = directory.join("config");
        config.paths.data_dir = directory.join("data");
        config.paths.state_dir = directory.join("state");
        config.paths.cache_dir = directory.join("cache");
        // `init_for_test` installs at most once per process; see
        // replication.rs's digest-mismatch test for why `init` would race.
        let tracing = crate::observability::init_for_test(&config.tracing, &config.telemetry)
            .expect("init the test tracing subscriber");
        AppState::build(config, tracing)
            .await
            .expect("build a real AppState backed by a temp dir")
    }

    // The tag reconcile is what keeps the store a cache: inventory added,
    // kept and removed must produce tag set, kept and removed — and a blob
    // file the registry does not list must never gain a tag, because the
    // registry is the only authority for what may be served.
    #[tokio::test]
    async fn the_mirror_tags_exactly_what_the_registry_holds() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let state = cluster_state(directory.path()).await;
        let blobs = FsStore::load(directory.path().join("cluster-blobs"))
            .await
            .expect("open the blob store");

        let first = state
            .registry()
            .put_object(
                "file".to_owned(),
                "text/plain".to_owned(),
                None,
                b"first object",
            )
            .expect("put the first object");
        let second = state
            .registry()
            .put_object(
                "file".to_owned(),
                "text/plain".to_owned(),
                None,
                b"second object",
            )
            .expect("put the second object");
        // A cached blob with no metadata is not an object the registry holds.
        let cached_id = format!("blake3:{}", blake3::hash(b"payload cache").to_hex());
        state
            .store()
            .cache_addressed(&cached_id, b"payload cache")
            .expect("cache a blob without metadata");

        let outcome = reconcile(&state, &blobs).await.expect("first reconcile");
        assert_eq!(
            outcome,
            ReconcileOutcome {
                imported: 2,
                kept: 0,
                removed: 0,
                failed: 0,
            }
        );
        assert!(tag_exists(&blobs, &first.id).await);
        assert!(tag_exists(&blobs, &second.id).await);
        assert!(
            !tag_exists(&blobs, &cached_id).await,
            "a blob the registry does not list must not gain a tag"
        );

        let outcome = reconcile(&state, &blobs)
            .await
            .expect("steady-state reconcile");
        assert_eq!(
            outcome,
            ReconcileOutcome {
                imported: 0,
                kept: 2,
                removed: 0,
                failed: 0,
            },
            "a second pass over an unchanged inventory is all keeps"
        );

        state
            .store()
            .remove_metadata(&second.id)
            .expect("the registry stops holding the second object");
        let outcome = reconcile(&state, &blobs).await.expect("removal reconcile");
        assert_eq!(
            outcome,
            ReconcileOutcome {
                imported: 0,
                kept: 1,
                removed: 1,
                failed: 0,
            }
        );
        assert!(tag_exists(&blobs, &first.id).await);
        assert!(
            !tag_exists(&blobs, &second.id).await,
            "a tag whose object the registry no longer holds must be deleted"
        );
    }
}
