//! `RegistryProvider` backed by an external kappa-registry instance.
//!
//! An object is one blob plus one sidecar OCI manifest. The blob carries the
//! bytes and, through its `Content-Type`, the media type. The manifest carries
//! the fields a content-addressed blob store has nowhere to put — kind,
//! filename, creation time — as annotations, and is tagged with the object's
//! kappa so a point lookup is a single deterministic fetch.

use super::kappa_client::{kappa_for, tag_for, KappaClient};
use super::RegistryProvider;
use crate::config::RegistryConfig;
use crate::error::{LiveError, Result};
use crate::protocol::{ObjectContent, ObjectMetadata, ObjectPage, ObjectQuery};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long a cached record is served without asking the registry again.
///
/// A tag is the hash of the bytes, so the blob a record describes never
/// changes. Its annotations can: a rename through this provider replaces the
/// entry at once, but a writer this daemon cannot see (another daemon on the
/// same store, or a token holder rewriting a manifest) is only noticed when
/// the entry expires. Five minutes bounds that window.
const METADATA_TTL: Duration = Duration::from_mins(5);

/// Upper bound on cached records, at a few hundred bytes each.
const METADATA_ENTRIES: usize = 50_000;

const ARTIFACT_TYPE: &str = "application/vnd.hologram.object.v1+json";
const ANNOTATION_KIND: &str = "dev.hologram.kind";
const ANNOTATION_FILENAME: &str = "dev.hologram.filename";
const ANNOTATION_CREATED: &str = "dev.hologram.created-at-millis";

pub struct KappaRegistryProvider {
    client: KappaClient,
    max_scan_pages: u32,
    /// Records by tag. Upstream cannot filter on kind or filename, so a
    /// search reads one manifest per stored object; without this every
    /// search paid that in full (measured: 2.8 s at 450 objects, one request
    /// a second under load). The tag walk itself is never cached, so an
    /// object written by anyone appears on the next search.
    metadata: Mutex<HashMap<String, (ObjectMetadata, Instant)>>,
    metadata_ttl: Duration,
}

impl KappaRegistryProvider {
    pub fn new(config: &RegistryConfig) -> Result<Self> {
        Ok(Self {
            client: KappaClient::new(config)?,
            max_scan_pages: config.max_scan_pages.max(1),
            metadata: Mutex::default(),
            metadata_ttl: METADATA_TTL,
        })
    }

    fn cached(&self, tag: &str) -> Option<ObjectMetadata> {
        let entries = self.metadata.lock().ok()?;
        let (metadata, stored) = entries.get(tag)?;
        (stored.elapsed() < self.metadata_ttl).then(|| metadata.clone())
    }

    fn remember(&self, tag: &str, metadata: &ObjectMetadata) {
        let Ok(mut entries) = self.metadata.lock() else {
            return;
        };
        if entries.len() >= METADATA_ENTRIES && !entries.contains_key(tag) {
            // Expired records go first; a cache that is still full is simply
            // started again. Either way the next search refills what it needs.
            let ttl = self.metadata_ttl;
            entries.retain(|_, (_, stored)| stored.elapsed() < ttl);
            if entries.len() >= METADATA_ENTRIES {
                entries.clear();
            }
        }
        entries.insert(tag.to_owned(), (metadata.clone(), Instant::now()));
    }

    /// Read one object's metadata by its tag. Returns `None` for a tag that is
    /// not ours, so an unrelated tag sharing the namespace is skipped rather
    /// than failing an entire listing.
    fn metadata_by_tag(&self, tag: &str) -> Result<Option<ObjectMetadata>> {
        if let Some(metadata) = self.cached(tag) {
            return Ok(Some(metadata));
        }
        match self.client.get_manifest(tag)? {
            Some(body) => {
                let metadata = decode_manifest(&body)?;
                self.remember(tag, &metadata);
                Ok(Some(metadata))
            }
            None => Ok(None),
        }
    }
}

/// Encode an object's metadata as its sidecar manifest.
fn encode_manifest(metadata: &ObjectMetadata) -> Result<Vec<u8>> {
    let mut annotations = serde_json::Map::new();
    annotations.insert(
        ANNOTATION_KIND.to_owned(),
        serde_json::Value::String(metadata.kind.clone()),
    );
    annotations.insert(
        ANNOTATION_CREATED.to_owned(),
        serde_json::Value::String(metadata.created_at_millis.to_string()),
    );
    if let Some(filename) = metadata.filename.as_ref() {
        annotations.insert(
            ANNOTATION_FILENAME.to_owned(),
            serde_json::Value::String(filename.clone()),
        );
    }

    let descriptor = serde_json::json!({
        "mediaType": metadata.media_type,
        "digest": metadata.id,
        "size": metadata.size,
    });
    let document = serde_json::json!({
        "schemaVersion": 2,
        "mediaType": "application/vnd.oci.image.manifest.v1+json",
        "artifactType": ARTIFACT_TYPE,
        "config": {
            "mediaType": "application/vnd.oci.empty.v1+json",
            "digest": metadata.id,
            "size": metadata.size,
        },
        "layers": [descriptor],
        "subject": descriptor,
        "annotations": serde_json::Value::Object(annotations),
    });
    serde_json::to_vec(&document).map_err(Into::into)
}

/// Decode a sidecar manifest, rejecting anything that is not ours rather than
/// returning a half-populated record.
fn decode_manifest(body: &[u8]) -> Result<ObjectMetadata> {
    let document: serde_json::Value = serde_json::from_slice(body)?;
    let annotations = document.get("annotations").and_then(|v| v.as_object());
    let annotation = |key: &str| -> Option<&str> {
        annotations
            .and_then(|map| map.get(key))
            .and_then(|value| value.as_str())
    };

    let subject = document
        .get("subject")
        .ok_or_else(|| LiveError::Protocol("object manifest has no subject".to_owned()))?;
    let id = subject
        .get("digest")
        .and_then(|value| value.as_str())
        .ok_or_else(|| LiveError::Protocol("object manifest subject has no digest".to_owned()))?;
    let kind = annotation(ANNOTATION_KIND).ok_or_else(|| {
        LiveError::Protocol(format!("object manifest is missing {ANNOTATION_KIND}"))
    })?;
    let created_at_millis = annotation(ANNOTATION_CREATED)
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or_else(|| {
            LiveError::Protocol(format!("object manifest is missing {ANNOTATION_CREATED}"))
        })?;
    let media_type = subject
        .get("mediaType")
        .and_then(|value| value.as_str())
        .unwrap_or("application/octet-stream");
    let size = subject
        .get("size")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or_default();

    Ok(ObjectMetadata {
        id: id.to_owned(),
        kind: kind.to_owned(),
        media_type: media_type.to_owned(),
        filename: annotation(ANNOTATION_FILENAME).map(str::to_owned),
        size,
        created_at_millis,
    })
}

impl RegistryProvider for KappaRegistryProvider {
    fn list_objects(&self, kind: Option<&str>) -> Result<Vec<ObjectMetadata>> {
        // Preserves the newest-first contract of the legacy surface.
        let query = ObjectQuery {
            kind: kind.map(str::to_owned),
            limit: ObjectQuery::MAX_LIMIT,
            ..ObjectQuery::default()
        };
        let mut objects = self.search(&query)?.objects;
        objects.sort_by(|left, right| {
            right
                .created_at_millis
                .cmp(&left.created_at_millis)
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(objects)
    }

    fn put_object(
        &self,
        kind: String,
        media_type: String,
        filename: Option<String>,
        bytes: &[u8],
    ) -> Result<ObjectMetadata> {
        let id = format!("blake3:{}", blake3::hash(bytes).to_hex());
        let tag = tag_for(&id);

        // The creation time of immutable content is the time it first
        // appeared, so an existing record wins over a fresh clock reading.
        let created_at_millis = match self.metadata_by_tag(&tag) {
            Ok(Some(existing)) => existing.created_at_millis,
            _ => crate::util::now_millis(),
        };

        let metadata = ObjectMetadata {
            id,
            kind,
            media_type,
            filename,
            size: bytes.len().try_into().unwrap_or(u64::MAX),
            created_at_millis,
        };

        self.client
            .put_blob(&metadata.id, &metadata.media_type, bytes)?;
        self.client
            .put_manifest(&tag, &encode_manifest(&metadata)?)?;
        self.remember(&tag, &metadata);
        Ok(metadata)
    }

    fn get_object(&self, id: &str) -> Result<ObjectContent> {
        let metadata = self
            .metadata_by_tag(&tag_for(id))?
            .ok_or_else(|| LiveError::NotFound(format!("object {id} not found")))?;
        let bytes = self.client.get_blob(id)?;
        Ok(ObjectContent { metadata, bytes })
    }

    fn rename_file(&self, id: &str, filename: String) -> Result<ObjectMetadata> {
        let tag = tag_for(id);
        let mut metadata = self
            .metadata_by_tag(&tag)?
            .ok_or_else(|| LiveError::NotFound(format!("file {id} not found")))?;
        if metadata.kind != "file" {
            return Err(LiveError::NotFound(format!("file {id} not found")));
        }
        metadata.filename = Some(filename);
        // The blob is immutable, so only the sidecar changes. Writing the same
        // tag replaces the record in one request, which avoids the
        // write-then-delete window a second tag would open.
        self.client
            .put_manifest(&tag, &encode_manifest(&metadata)?)?;
        self.remember(&tag, &metadata);
        Ok(metadata)
    }

    fn search(&self, query: &ObjectQuery) -> Result<ObjectPage> {
        let limit = query.effective_limit();
        let mut objects: Vec<ObjectMetadata> = Vec::with_capacity(limit);
        let mut cursor = query.cursor.as_deref().map(tag_for);
        let mut truncated = false;
        let mut pages = 0_u32;

        loop {
            if pages == self.max_scan_pages {
                // Upstream cannot filter on kind or filename, so a selective
                // query walks pages here. Report the bound instead of
                // presenting a capped result as a complete one.
                truncated = true;
                tracing::debug!(
                    pages,
                    found = objects.len(),
                    "kappa registry search stopped at the page bound"
                );
                break;
            }
            let page = self
                .client
                .list_tags(ObjectQuery::MAX_LIMIT, cursor.as_deref())?;
            pages += 1;
            if page.tags.is_empty() {
                break;
            }
            for tag in &page.tags {
                if kappa_for(tag).is_none() {
                    continue;
                }
                let Some(metadata) = self.metadata_by_tag(tag)? else {
                    continue;
                };
                if !query.matches(&metadata) {
                    continue;
                }
                if objects.len() == limit {
                    return Ok(ObjectPage {
                        next_cursor: objects.last().map(|object| object.id.clone()),
                        objects,
                        truncated: false,
                    });
                }
                objects.push(metadata);
            }
            match page.next {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }

        let next_cursor = if truncated {
            objects.last().map(|object| object.id.clone())
        } else {
            None
        };
        Ok(ObjectPage {
            objects,
            next_cursor,
            truncated,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    /// A registry small enough to count: tags, manifests and nothing else.
    /// `manifest_reads` is the number the cache exists to bring down.
    struct FakeRegistry {
        endpoint: String,
        manifests: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
        manifest_reads: Arc<AtomicUsize>,
    }

    impl FakeRegistry {
        fn start() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
            let endpoint = format!("http://{}", listener.local_addr().expect("addr"));
            let manifests: Arc<Mutex<BTreeMap<String, Vec<u8>>>> = Arc::default();
            let manifest_reads = Arc::new(AtomicUsize::new(0));
            let (store, reads) = (manifests.clone(), manifest_reads.clone());
            std::thread::spawn(move || {
                for stream in listener.incoming().flatten() {
                    let mut reader = BufReader::new(stream);
                    let mut line = String::new();
                    if reader.read_line(&mut line).is_err() {
                        continue;
                    }
                    let mut parts = line.split_whitespace();
                    let (method, target) = (
                        parts.next().unwrap_or("").to_owned(),
                        parts.next().unwrap_or("").to_owned(),
                    );
                    let mut length = 0_usize;
                    loop {
                        let mut header = String::new();
                        if reader.read_line(&mut header).unwrap_or(0) == 0 || header == "\r\n" {
                            break;
                        }
                        if let Some(value) =
                            header.to_ascii_lowercase().strip_prefix("content-length:")
                        {
                            length = value.trim().parse().unwrap_or(0);
                        }
                    }
                    let mut body = vec![0_u8; length];
                    let _ = reader.read_exact(&mut body);
                    let path = target.split('?').next().unwrap_or("");
                    let (status, payload): (&str, Vec<u8>) = if path.ends_with("/tags/list") {
                        let tags: Vec<String> =
                            store.lock().expect("lock").keys().cloned().collect();
                        (
                            "200 OK",
                            serde_json::to_vec(&serde_json::json!({ "tags": tags })).expect("json"),
                        )
                    } else if let Some(tag) = path
                        .rsplit_once("/manifests/")
                        .map(|(_, tag)| tag.to_owned())
                    {
                        if method == "PUT" {
                            store.lock().expect("lock").insert(tag, body);
                            ("201 Created", Vec::new())
                        } else {
                            reads.fetch_add(1, Ordering::SeqCst);
                            match store.lock().expect("lock").get(&tag) {
                                Some(manifest) => ("200 OK", manifest.clone()),
                                None => ("404 Not Found", Vec::new()),
                            }
                        }
                    } else if method == "PUT" {
                        ("201 Created", Vec::new())
                    } else {
                        ("404 Not Found", Vec::new())
                    };
                    let mut stream = reader.into_inner();
                    let _ = write!(
                        stream,
                        "HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                        payload.len()
                    );
                    let _ = stream.write_all(&payload);
                }
            });
            Self {
                endpoint,
                manifests,
                manifest_reads,
            }
        }

        fn provider(&self) -> KappaRegistryProvider {
            KappaRegistryProvider::new(&RegistryConfig {
                provider: "kappa".to_owned(),
                endpoint: self.endpoint.clone(),
                namespace: "cache-test".to_owned(),
                ..RegistryConfig::default()
            })
            .expect("provider")
        }

        fn reads(&self) -> usize {
            self.manifest_reads.swap(0, Ordering::SeqCst)
        }
    }

    fn put(provider: &KappaRegistryProvider, name: &str) -> ObjectMetadata {
        provider
            .put_object(
                "file".into(),
                "text/plain".into(),
                Some(name.into()),
                name.as_bytes(),
            )
            .expect("put")
    }

    #[test]
    fn a_repeated_search_reads_no_manifest_twice() {
        let registry = FakeRegistry::start();
        let provider = registry.provider();
        for name in ["a.txt", "b.txt", "c.txt"] {
            put(&provider, name);
        }
        registry.reads();

        let first = provider
            .search(&ObjectQuery::default())
            .expect("first search");
        assert_eq!(first.objects.len(), 3);
        registry.reads();

        let second = provider
            .search(&ObjectQuery::default())
            .expect("second search");
        let ids = |page: &ObjectPage| -> Vec<String> {
            page.objects
                .iter()
                .map(|object| object.id.clone())
                .collect()
        };
        assert_eq!(ids(&second), ids(&first));
        assert_eq!(registry.reads(), 0, "every manifest was read a moment ago");
    }

    #[test]
    fn an_expired_record_is_read_again() {
        let registry = FakeRegistry::start();
        let mut provider = registry.provider();
        provider.metadata_ttl = Duration::ZERO;
        put(&provider, "a.txt");
        provider.search(&ObjectQuery::default()).expect("first");
        registry.reads();

        provider.search(&ObjectQuery::default()).expect("second");
        assert_eq!(
            registry.reads(),
            1,
            "a rewrite this daemon cannot see converges when the record expires"
        );
    }

    #[test]
    fn a_rename_is_visible_at_once() {
        let registry = FakeRegistry::start();
        let provider = registry.provider();
        let stored = put(&provider, "before.txt");
        provider.search(&ObjectQuery::default()).expect("warm");

        provider
            .rename_file(&stored.id, "after.txt".to_owned())
            .expect("rename");
        let page = provider.search(&ObjectQuery::default()).expect("search");
        assert_eq!(page.objects[0].filename.as_deref(), Some("after.txt"));
    }

    #[test]
    fn a_tag_written_behind_the_providers_back_appears_on_the_next_search() {
        let registry = FakeRegistry::start();
        let provider = registry.provider();
        put(&provider, "mine.txt");
        provider.search(&ObjectQuery::default()).expect("warm");

        // Another writer: the CLI pushing straight to the registry, or a second daemon on the same store.
        let mut foreign = sample();
        foreign.filename = Some("theirs.txt".to_owned());
        registry.manifests.lock().expect("lock").insert(
            tag_for(&foreign.id),
            encode_manifest(&foreign).expect("encode"),
        );

        let page = provider.search(&ObjectQuery::default()).expect("search");
        assert_eq!(
            page.objects.len(),
            2,
            "the tag walk is never cached, only the records"
        );
        assert!(page
            .objects
            .iter()
            .any(|object| object.filename.as_deref() == Some("theirs.txt")));
    }

    fn sample() -> ObjectMetadata {
        ObjectMetadata {
            id: "blake3:cb9ef1526f722fcaaf5a6e19610d045349627e4ce70ad4420ccc00b6bfc5e959"
                .to_owned(),
            kind: "file".to_owned(),
            media_type: "text/plain".to_owned(),
            filename: Some("notes.txt".to_owned()),
            size: 24,
            created_at_millis: 1_757_894_400_000,
        }
    }

    #[test]
    fn metadata_round_trips_through_a_sidecar_manifest() {
        let original = sample();
        let encoded = encode_manifest(&original).expect("encode");
        let decoded = decode_manifest(&encoded).expect("decode");

        assert_eq!(decoded.id, original.id);
        assert_eq!(decoded.kind, original.kind);
        assert_eq!(decoded.media_type, original.media_type);
        assert_eq!(decoded.filename, original.filename);
        assert_eq!(decoded.size, original.size);
        assert_eq!(decoded.created_at_millis, original.created_at_millis);
    }

    #[test]
    fn the_manifest_subject_is_the_blob_it_describes() {
        let original = sample();
        let encoded = encode_manifest(&original).expect("encode");
        let document: serde_json::Value = serde_json::from_slice(&encoded).expect("json");

        assert_eq!(
            document["subject"]["digest"].as_str(),
            Some(original.id.as_str()),
            "subject points at the blob, which is what the referrers API indexes"
        );
        assert_eq!(
            document["artifactType"].as_str(),
            Some("application/vnd.hologram.object.v1+json")
        );
    }

    #[test]
    fn a_manifest_missing_our_annotations_is_rejected() {
        let foreign = br#"{"schemaVersion":2,"mediaType":"application/vnd.oci.image.manifest.v1+json","layers":[]}"#;
        assert!(
            decode_manifest(foreign).is_err(),
            "a manifest that is not ours must not decode to a half-populated object"
        );
    }

    #[test]
    fn a_missing_filename_round_trips_as_absent() {
        let mut original = sample();
        original.filename = None;
        let decoded =
            decode_manifest(&encode_manifest(&original).expect("encode")).expect("decode");
        assert_eq!(decoded.filename, None);
    }
}
