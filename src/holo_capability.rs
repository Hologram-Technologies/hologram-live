use crate::error::{LiveError, Result};
use hologram::space::{Capabilities, CapabilitySet, KappaLabel71, Realization};
use serde::{Deserialize, Serialize};
use std::path::Path;

const SOURCE_SCHEMA_VERSION: u16 = 1;
const BLAKE3_LABEL_LENGTH: usize = 71;

/// Human-authored capability request compiled into the upstream canonical
/// `CapabilitySet` realization stored in a `.holo` archive.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CapabilitySource {
    pub schema_version: u16,
    pub storage_roots: Vec<String>,
    pub storage_quota_bytes: u64,
    pub network_fetch: bool,
    pub network_announce: bool,
    pub publish_channels: Vec<String>,
    pub subscribe_channels: Vec<String>,
    pub memory_max_bytes: u64,
    pub cpu_time_per_event_ms: u64,
    pub priority_weight: u32,
}

impl Default for CapabilitySource {
    fn default() -> Self {
        Self {
            schema_version: SOURCE_SCHEMA_VERSION,
            storage_roots: Vec::new(),
            storage_quota_bytes: 0,
            network_fetch: false,
            network_announce: false,
            publish_channels: Vec::new(),
            subscribe_channels: Vec::new(),
            memory_max_bytes: 0,
            cpu_time_per_event_ms: 0,
            priority_weight: 0,
        }
    }
}

/// Compile a source JSON document into canonical runtime bytes.
pub fn compile_source(path: &Path, source: &[u8]) -> Result<Vec<u8>> {
    let document: CapabilitySource = serde_json::from_slice(source).map_err(|error| {
        LiveError::Config(format!(
            "parse capability source {}: {error}",
            path.display()
        ))
    })?;
    document.canonicalize(path)
}

/// Canonical empty request used when `hologram.json` omits `requires`.
pub fn empty_canonical() -> Vec<u8> {
    CapabilitySet::new(empty_capabilities()).canonicalize()
}

/// Decode a runtime object and prove it is exactly a canonical `CapabilitySet`.
pub fn decode_canonical(bytes: &[u8]) -> Result<Capabilities> {
    let capabilities = CapabilitySet::to_capabilities(bytes).map_err(|error| {
        LiveError::InvalidHolo(format!("decode required CapabilitySet: {error:?}"))
    })?;
    if CapabilitySet::new(capabilities.clone()).canonicalize() != bytes {
        return Err(LiveError::InvalidHolo(
            "required CapabilitySet is not canonically encoded".to_owned(),
        ));
    }
    Ok(capabilities)
}

pub fn empty_capabilities() -> Capabilities {
    Capabilities {
        storage_roots: Vec::new(),
        storage_quota_bytes: 0,
        network_fetch: false,
        network_announce: false,
        publish_channels: Vec::new(),
        subscribe_channels: Vec::new(),
        memory_max_bytes: 0,
        cpu_time_per_event_ms: 0,
        priority_weight: 0,
    }
}

impl CapabilitySource {
    fn canonicalize(self, path: &Path) -> Result<Vec<u8>> {
        if self.schema_version != SOURCE_SCHEMA_VERSION {
            return Err(source_error(
                path,
                "schema_version",
                &format!(
                    "unsupported value {}; expected {SOURCE_SCHEMA_VERSION}",
                    self.schema_version
                ),
            ));
        }
        let capabilities = Capabilities {
            storage_roots: parse_labels(path, "storage_roots", &self.storage_roots)?,
            storage_quota_bytes: self.storage_quota_bytes,
            network_fetch: self.network_fetch,
            network_announce: self.network_announce,
            publish_channels: parse_labels(path, "publish_channels", &self.publish_channels)?,
            subscribe_channels: parse_labels(path, "subscribe_channels", &self.subscribe_channels)?,
            memory_max_bytes: self.memory_max_bytes,
            cpu_time_per_event_ms: self.cpu_time_per_event_ms,
            priority_weight: self.priority_weight,
        };
        Ok(CapabilitySet::new(capabilities).canonicalize())
    }
}

fn parse_labels(path: &Path, field: &str, values: &[String]) -> Result<Vec<KappaLabel71>> {
    for (index, pair) in values.windows(2).enumerate() {
        if pair[0] >= pair[1] {
            let reason = if pair[0] == pair[1] {
                "duplicates the previous value"
            } else {
                "is not in ascending lexical order"
            };
            return Err(source_error(
                path,
                &format!("{field}[{}]", index + 1),
                reason,
            ));
        }
    }
    values
        .iter()
        .enumerate()
        .map(|(index, value)| parse_label(path, &format!("{field}[{index}]"), value))
        .collect()
}

fn parse_label(path: &Path, field: &str, value: &str) -> Result<KappaLabel71> {
    let valid = value.len() == BLAKE3_LABEL_LENGTH
        && value.starts_with("blake3:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    if !valid {
        return Err(source_error(
            path,
            field,
            "expected canonical blake3:<64 lowercase hex> κ",
        ));
    }
    KappaLabel71::from_bytes(value.as_bytes())
        .map_err(|error| source_error(path, field, &format!("invalid κ label: {error:?}")))
}

fn source_error(path: &Path, field: &str, message: &str) -> LiveError {
    LiveError::Config(format!(
        "capability source {} field {field}: {message}",
        path.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use hologram::space::address_bytes;

    #[test]
    fn source_compiles_to_upstream_canonical_realization() {
        let first = address_bytes(b"first").to_string();
        let second = address_bytes(b"second").to_string();
        let (lower, upper) = if first < second {
            (first, second)
        } else {
            (second, first)
        };
        let source = format!(
            r#"{{
                "schema_version": 1,
                "storage_roots": ["{lower}", "{upper}"],
                "storage_quota_bytes": 4096,
                "network_fetch": true,
                "memory_max_bytes": 1048576,
                "cpu_time_per_event_ms": 50,
                "priority_weight": 2
            }}"#
        );
        let bytes = compile_source(Path::new("capabilities.json"), source.as_bytes())
            .expect("compile capability source");
        let decoded = decode_canonical(&bytes).expect("decode canonical capabilities");
        assert_eq!(decoded.storage_roots.len(), 2);
        assert_eq!(decoded.storage_quota_bytes, 4096);
        assert!(decoded.network_fetch);
        assert_eq!(decoded.memory_max_bytes, 1_048_576);
        assert_eq!(decoded.cpu_time_per_event_ms, 50);
        assert_eq!(decoded.priority_weight, 2);
    }

    #[test]
    fn source_rejects_noncanonical_and_duplicate_labels_with_locations() {
        let path = Path::new("app/capabilities.json");
        let invalid = br#"{"storage_roots":["BLake3:not-a-kappa"]}"#;
        let error = compile_source(path, invalid).expect_err("invalid label");
        assert!(error.to_string().contains("storage_roots[0]"), "{error}");
        assert!(
            error.to_string().contains("app/capabilities.json"),
            "{error}"
        );

        let label = address_bytes(b"same").to_string();
        let duplicate = format!(r#"{{"storage_roots":["{label}","{label}"]}}"#);
        let error = compile_source(path, duplicate.as_bytes()).expect_err("duplicate label");
        assert!(error.to_string().contains("storage_roots[1]"), "{error}");
        assert!(error.to_string().contains("duplicates"), "{error}");
    }

    #[test]
    fn source_rejects_unknown_fields_versions_and_unstable_order() {
        let path = Path::new("capabilities.json");
        let error =
            compile_source(path, br#"{"ambient_authority":true}"#).expect_err("unknown field");
        assert!(error.to_string().contains("ambient_authority"), "{error}");

        let error =
            compile_source(path, br#"{"schema_version":2}"#).expect_err("unsupported version");
        assert!(error.to_string().contains("schema_version"), "{error}");

        let low = address_bytes(b"low").to_string();
        let high = address_bytes(b"high").to_string();
        let (lower, upper) = if low < high { (low, high) } else { (high, low) };
        let source = format!(r#"{{"publish_channels":["{upper}","{lower}"]}}"#);
        let error = compile_source(path, source.as_bytes()).expect_err("unstable order");
        assert!(
            error.to_string().contains("ascending lexical order"),
            "{error}"
        );
    }

    #[test]
    fn empty_source_and_omitted_source_are_identical() {
        let explicit = compile_source(Path::new("capabilities.json"), b"{}")
            .expect("explicit empty capabilities");
        assert_eq!(explicit, empty_canonical());
        assert_eq!(
            decode_canonical(&explicit).expect("decode"),
            empty_capabilities()
        );
    }
}
