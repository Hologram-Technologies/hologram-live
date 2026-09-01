//! Bounded resolution of typed UOR storage graphs.

use crate::error::{LiveError, Result};
use crate::store::ObjectStore;
use hologram::space::{address_bytes, KappaLabel71, RealizationError, REGISTRY};
use std::collections::{HashSet, VecDeque};

const KAPPA_LABEL_BYTES: usize = 71;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StorageGraphLimits {
    pub max_depth: usize,
    pub max_objects: usize,
    pub max_edges: usize,
    pub max_resolved_bytes: u64,
}

impl Default for StorageGraphLimits {
    fn default() -> Self {
        Self {
            max_depth: 16,
            max_objects: 512,
            max_edges: 4_096,
            max_resolved_bytes: 64 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageGraphClosure {
    /// Breadth-first, first-seen readable objects, including every root.
    pub readable: Vec<String>,
    pub edge_count: usize,
    pub resolved_bytes: u64,
    pub max_depth: usize,
}

/// Resolve the complete local typed closure of admitted storage roots.
///
/// Unknown or untagged objects are opaque leaves. A recognized realization IRI
/// must use the canonical UOR frame and every referenced object must be present
/// and match its κ. Resolution is deliberately local and fail-closed: it never
/// consults a network or turns a partial closure into authority.
pub fn resolve_storage_graph(
    store: &ObjectStore,
    roots: &[KappaLabel71],
    limits: StorageGraphLimits,
) -> Result<StorageGraphClosure> {
    if roots.is_empty() {
        return Err(graph_error("requires at least one storage root"));
    }
    if limits.max_objects == 0 || limits.max_edges == 0 || limits.max_resolved_bytes == 0 {
        return Err(graph_error("has an invalid zero host limit"));
    }

    let mut queue = VecDeque::new();
    for root in roots {
        queue.push_back((root.to_string(), 0usize));
    }
    let mut seen = HashSet::new();
    let mut readable = Vec::new();
    let mut edge_count = 0usize;
    let mut resolved_bytes = 0u64;
    let mut observed_depth = 0usize;

    while let Some((kappa, depth)) = queue.pop_front() {
        if !seen.insert(kappa.clone()) {
            continue;
        }
        if readable.len() >= limits.max_objects {
            return Err(graph_error("exceeds the host object-count limit"));
        }
        if depth > limits.max_depth {
            return Err(graph_error("exceeds the host depth limit"));
        }

        let bytes = store.get(&kappa).map_err(|error| match error {
            LiveError::NotFound(_) => graph_error("is incomplete in the local object store"),
            _ => graph_error("could not be read from the local object store"),
        })?;
        if address_bytes(&bytes).to_string() != kappa {
            return Err(graph_error(
                "contains an object whose bytes do not match its κ",
            ));
        }
        let byte_count = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        resolved_bytes = resolved_bytes
            .checked_add(byte_count)
            .ok_or_else(|| graph_error("exceeds the host byte limit"))?;
        if resolved_bytes > limits.max_resolved_bytes {
            return Err(graph_error("exceeds the host byte limit"));
        }

        let references = typed_references(&bytes)?;
        edge_count = edge_count
            .checked_add(references.len())
            .ok_or_else(|| graph_error("exceeds the host edge-count limit"))?;
        if edge_count > limits.max_edges {
            return Err(graph_error("exceeds the host edge-count limit"));
        }
        if depth == limits.max_depth && !references.is_empty() {
            return Err(graph_error("exceeds the host depth limit"));
        }
        for reference in references {
            queue.push_back((reference.to_string(), depth + 1));
        }

        observed_depth = observed_depth.max(depth);
        readable.push(kappa);
    }

    Ok(StorageGraphClosure {
        readable,
        edge_count,
        resolved_bytes,
        max_depth: observed_depth,
    })
}

fn typed_references(bytes: &[u8]) -> Result<Vec<KappaLabel71>> {
    let Some(nul) = bytes.iter().position(|byte| *byte == 0) else {
        return Ok(Vec::new());
    };
    let Ok(iri) = std::str::from_utf8(&bytes[..nul]) else {
        return Ok(Vec::new());
    };
    let Some((_, extractor)) = REGISTRY.iter().find(|(registered, _)| *registered == iri) else {
        return Ok(Vec::new());
    };
    let references = extractor(bytes).map_err(|error| typed_error(iri, error))?;
    validate_canonical_frame(bytes, nul, references.len())
        .map_err(|error| typed_error(iri, error))?;
    Ok(references)
}

fn validate_canonical_frame(
    bytes: &[u8],
    nul: usize,
    extracted_references: usize,
) -> std::result::Result<(), RealizationError> {
    let mut cursor = nul.checked_add(1).ok_or(RealizationError::Truncated)?;
    let count = read_u32(bytes, &mut cursor)? as usize;
    if count != extracted_references {
        return Err(RealizationError::Malformed);
    }
    cursor = cursor
        .checked_add(
            count
                .checked_mul(KAPPA_LABEL_BYTES)
                .ok_or(RealizationError::Truncated)?,
        )
        .ok_or(RealizationError::Truncated)?;
    let payload_length = read_u32(bytes, &mut cursor)? as usize;
    let end = cursor
        .checked_add(payload_length)
        .ok_or(RealizationError::Truncated)?;
    if end != bytes.len() {
        return Err(if end > bytes.len() {
            RealizationError::Truncated
        } else {
            RealizationError::Malformed
        });
    }
    Ok(())
}

fn read_u32(bytes: &[u8], cursor: &mut usize) -> std::result::Result<u32, RealizationError> {
    let end = cursor.checked_add(4).ok_or(RealizationError::Truncated)?;
    let value = bytes
        .get(*cursor..end)
        .ok_or(RealizationError::Truncated)?
        .try_into()
        .map(u32::from_le_bytes)
        .map_err(|_| RealizationError::Truncated)?;
    *cursor = end;
    Ok(value)
}

fn typed_error(iri: &str, error: RealizationError) -> LiveError {
    tracing::warn!(
        realization_iri = iri,
        ?error,
        "rejected malformed typed storage object"
    );
    graph_error("contains a malformed typed realization")
}

fn graph_error(message: &str) -> LiveError {
    LiveError::Capability(format!("typed storage graph {message}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use hologram::space::{address_bytes, Channel, Realization, Route};
    use std::sync::Arc;

    fn store() -> (tempfile::TempDir, Arc<ObjectStore>) {
        let directory = tempfile::tempdir().expect("tempdir");
        let store = Arc::new(ObjectStore::open(directory.path()).expect("store"));
        (directory, store)
    }

    fn cache(store: &ObjectStore, bytes: &[u8]) -> KappaLabel71 {
        let kappa = address_bytes(bytes);
        store
            .cache_addressed(kappa.as_ref(), bytes)
            .expect("cache object");
        kappa
    }

    #[test]
    fn resolves_typed_edges_and_treats_opaque_objects_as_leaves() {
        let (_directory, store) = store();
        let leaf = cache(&store, b"opaque leaf");
        let channel = Channel {
            type_shape: Some(leaf),
            decl_payload: b"typed parent".to_vec(),
        }
        .canonicalize();
        let root = cache(&store, &channel);

        let closure = resolve_storage_graph(&store, &[root], StorageGraphLimits::default())
            .expect("resolve closure");
        assert_eq!(closure.readable, [root.to_string(), leaf.to_string()]);
        assert_eq!(closure.edge_count, 1);
        assert_eq!(closure.max_depth, 1);
    }

    #[test]
    fn deduplicates_repeated_edges_in_first_seen_order() {
        let (_directory, store) = store();
        let leaf = cache(&store, b"shared leaf");
        let route = Route {
            endpoint: leaf,
            target: leaf,
        }
        .canonicalize();
        let root = cache(&store, &route);
        let closure = resolve_storage_graph(&store, &[root], StorageGraphLimits::default())
            .expect("resolve closure");
        assert_eq!(closure.readable, [root.to_string(), leaf.to_string()]);
        assert_eq!(closure.edge_count, 2);
    }

    #[test]
    fn rejects_missing_members_and_malformed_claimed_types_without_leaking_kappas() {
        let (_directory, store) = store();
        let missing = address_bytes(b"missing");
        let root = cache(
            &store,
            &Channel {
                type_shape: Some(missing),
                decl_payload: Vec::new(),
            }
            .canonicalize(),
        );
        let error = resolve_storage_graph(&store, &[root], StorageGraphLimits::default())
            .expect_err("missing member");
        assert_eq!(error.code(), "LIVE_CAPABILITY_MISSING");
        assert!(!error.to_string().contains(&missing.to_string()));

        let malformed = format!("{}\0", Channel::IRI).into_bytes();
        let malformed_root = cache(&store, &malformed);
        let error = resolve_storage_graph(&store, &[malformed_root], StorageGraphLimits::default())
            .expect_err("malformed typed object");
        assert!(error.to_string().contains("malformed typed realization"));
        assert!(!error.to_string().contains(&malformed_root.to_string()));
    }

    #[test]
    fn enforces_depth_object_edge_and_byte_limits() {
        let (_directory, store) = store();
        let leaf = cache(&store, b"leaf");
        let middle = cache(
            &store,
            &Channel {
                type_shape: Some(leaf),
                decl_payload: Vec::new(),
            }
            .canonicalize(),
        );
        let root = cache(
            &store,
            &Channel {
                type_shape: Some(middle),
                decl_payload: Vec::new(),
            }
            .canonicalize(),
        );
        let base = StorageGraphLimits::default();

        for (limits, expected) in [
            (
                StorageGraphLimits {
                    max_depth: 1,
                    ..base
                },
                "depth",
            ),
            (
                StorageGraphLimits {
                    max_objects: 2,
                    ..base
                },
                "object-count",
            ),
            (
                StorageGraphLimits {
                    max_edges: 1,
                    ..base
                },
                "edge-count",
            ),
            (
                StorageGraphLimits {
                    max_resolved_bytes: 1,
                    ..base
                },
                "byte",
            ),
        ] {
            let error = resolve_storage_graph(&store, &[root], limits).expect_err("limit");
            assert!(error.to_string().contains(expected), "{error}");
        }
    }
}
