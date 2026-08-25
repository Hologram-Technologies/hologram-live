use crate::error::{LiveError, Result};
use hologram::space::{Capabilities, CapabilitySet, KappaLabel71, Realization};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;

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

#[derive(Debug, Clone)]
pub struct RequestedCapabilities {
    pub kappa: String,
    pub canonical: Arc<[u8]>,
    pub capabilities: Arc<Capabilities>,
}

impl RequestedCapabilities {
    pub fn decode(kappa: &str, bytes: Arc<[u8]>) -> Result<Self> {
        if hologram::space::address_bytes(&bytes) != kappa {
            return Err(LiveError::InvalidHolo(format!(
                "required CapabilitySet bytes do not match declared κ {kappa}"
            )));
        }
        let capabilities = Arc::new(decode_canonical(&bytes)?);
        Ok(Self {
            kappa: kappa.to_owned(),
            canonical: bytes,
            capabilities,
        })
    }
}

/// Capability authority named by a parent-to-child application edge.
///
/// Archive bytes only become authority after the runtime proves this set is an
/// attenuation of the parent's already trusted effective grant.
#[derive(Debug, Clone)]
pub struct DelegatedCapabilities {
    pub kappa: String,
    pub canonical: Arc<[u8]>,
    pub capabilities: Arc<Capabilities>,
}

impl DelegatedCapabilities {
    pub fn decode(kappa: &str, bytes: Arc<[u8]>) -> Result<Self> {
        if hologram::space::address_bytes(&bytes) != kappa {
            return Err(LiveError::InvalidHolo(format!(
                "delegated CapabilitySet bytes do not match declared κ {kappa}"
            )));
        }
        let capabilities = Arc::new(decode_canonical(&bytes)?);
        Ok(Self {
            kappa: kappa.to_owned(),
            canonical: bytes,
            capabilities,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantSource {
    LocalBaseline,
    DirectDevelopmentFile,
    ServiceDevelopmentFile,
}

impl GrantSource {
    pub const fn name(self) -> &'static str {
        match self {
            Self::LocalBaseline => "local_baseline",
            Self::DirectDevelopmentFile => "direct_development_file",
            Self::ServiceDevelopmentFile => "service_development_file",
        }
    }
}

/// Authority supplied by a trusted execution context, never by an archive.
#[derive(Debug, Clone)]
pub struct EffectiveGrant {
    pub kappa: String,
    pub canonical: Arc<[u8]>,
    pub capabilities: Arc<Capabilities>,
    pub source: GrantSource,
}

impl EffectiveGrant {
    pub fn local_baseline() -> Self {
        let canonical = empty_canonical();
        Self {
            kappa: hologram::space::address_bytes(&canonical).to_string(),
            canonical: Arc::from(canonical),
            capabilities: Arc::new(empty_capabilities()),
            source: GrantSource::LocalBaseline,
        }
    }

    pub fn from_development_file(path: &Path, source: GrantSource) -> Result<Self> {
        if source == GrantSource::LocalBaseline {
            return Err(LiveError::Config(
                "a development grant file requires a development grant source".to_owned(),
            ));
        }
        let bytes = std::fs::read(path).map_err(|error| LiveError::io(path, error))?;
        Self::from_canonical(compile_source(path, &bytes)?, source)
    }

    pub fn authorize(
        &self,
        application_kappa: &str,
        request: &RequestedCapabilities,
    ) -> Result<()> {
        if self.capabilities.admits(&request.capabilities) {
            tracing::info!(
                application_kappa,
                requested_capabilities_kappa = %request.kappa,
                effective_grant_kappa = %self.kappa,
                grant_source = self.source.name(),
                capability_decision = "allow",
                "authorized holo application capabilities"
            );
            return Ok(());
        }
        tracing::warn!(
            application_kappa,
            requested_capabilities_kappa = %request.kappa,
            effective_grant_kappa = %self.kappa,
            grant_source = self.source.name(),
            capability_decision = "deny",
            "denied holo application capabilities"
        );
        Err(LiveError::Authorization(format!(
            "application {application_kappa} requests capabilities {} ({}) not admitted by effective grant {} from {} ({})",
            request.kappa,
            summary(&request.capabilities),
            self.kappa,
            self.source.name(),
            summary(&self.capabilities)
        )))
    }

    fn from_canonical(bytes: Vec<u8>, source: GrantSource) -> Result<Self> {
        let capabilities = Arc::new(decode_canonical(&bytes)?);
        Ok(Self {
            kappa: hologram::space::address_bytes(&bytes).to_string(),
            canonical: Arc::from(bytes),
            capabilities,
            source,
        })
    }
}

pub fn authorize_child_delegation(
    parent_application_kappa: &str,
    parent_grant_kappa: &str,
    parent_grant: &Capabilities,
    child_application_kappa: &str,
    delegated: &DelegatedCapabilities,
    request: &RequestedCapabilities,
) -> Result<()> {
    if !parent_grant.admits(&delegated.capabilities) {
        tracing::warn!(
            parent_application_kappa,
            child_application_kappa,
            parent_grant_kappa,
            delegated_capabilities_kappa = %delegated.kappa,
            capability_decision = "deny",
            capability_relation = "delegation",
            "denied child capability amplification"
        );
        return Err(LiveError::Authorization(format!(
            "application {parent_application_kappa} delegates capabilities {} ({}) to child {child_application_kappa}, which is not admitted by parent grant {parent_grant_kappa} ({})",
            delegated.kappa,
            summary(&delegated.capabilities),
            summary(parent_grant)
        )));
    }
    if !delegated.capabilities.admits(&request.capabilities) {
        tracing::warn!(
            parent_application_kappa,
            child_application_kappa,
            delegated_capabilities_kappa = %delegated.kappa,
            requested_capabilities_kappa = %request.kappa,
            capability_decision = "deny",
            capability_relation = "child_request",
            "denied under-granted child capability request"
        );
        return Err(LiveError::Authorization(format!(
            "child application {child_application_kappa} requests capabilities {} ({}) not admitted by delegated grant {} ({}) from parent {parent_application_kappa}",
            request.kappa,
            summary(&request.capabilities),
            delegated.kappa,
            summary(&delegated.capabilities)
        )));
    }
    tracing::info!(
        parent_application_kappa,
        child_application_kappa,
        parent_grant_kappa,
        delegated_capabilities_kappa = %delegated.kappa,
        requested_capabilities_kappa = %request.kappa,
        capability_decision = "allow",
        capability_relation = "child_attenuation",
        "authorized child capability attenuation"
    );
    Ok(())
}

fn summary(capabilities: &Capabilities) -> String {
    format!(
        "storage_roots={}, publish_channels={}, subscribe_channels={}, network_fetch={}, network_announce={}, storage_quota_bytes={}, memory_max_bytes={}, cpu_time_per_event_ms={}, priority_weight={}",
        capabilities.storage_roots.len(),
        capabilities.publish_channels.len(),
        capabilities.subscribe_channels.len(),
        capabilities.network_fetch,
        capabilities.network_announce,
        capabilities.storage_quota_bytes,
        capabilities.memory_max_bytes,
        capabilities.cpu_time_per_event_ms,
        capabilities.priority_weight
    )
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

    #[test]
    fn baseline_authorizes_empty_requests_and_denies_network_authority() {
        let baseline = EffectiveGrant::local_baseline();
        let empty = Arc::<[u8]>::from(empty_canonical());
        let request =
            RequestedCapabilities::decode(hologram::space::address_bytes(&empty).as_ref(), empty)
                .expect("empty request");
        baseline
            .authorize("blake3:application", &request)
            .expect("baseline admits empty request");

        let source = br#"{"network_fetch":true}"#;
        let bytes = Arc::<[u8]>::from(
            compile_source(Path::new("network.json"), source).expect("network request"),
        );
        let request =
            RequestedCapabilities::decode(hologram::space::address_bytes(&bytes).as_ref(), bytes)
                .expect("request");
        let error = baseline
            .authorize("blake3:application", &request)
            .expect_err("baseline denies network fetch");
        assert_eq!(error.code(), "LIVE_AUTHORIZATION_DENIED");
        assert!(error.to_string().contains("network_fetch=true"), "{error}");
    }

    #[test]
    fn explicit_development_grant_uses_upstream_attenuation() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("grant.json");
        std::fs::write(&path, r#"{"network_fetch":true}"#).expect("grant");
        let grant =
            EffectiveGrant::from_development_file(&path, GrantSource::DirectDevelopmentFile)
                .expect("development grant");
        let bytes = Arc::<[u8]>::from(
            compile_source(Path::new("request.json"), br#"{"network_fetch":true}"#)
                .expect("request"),
        );
        let request =
            RequestedCapabilities::decode(hologram::space::address_bytes(&bytes).as_ref(), bytes)
                .expect("request");
        grant
            .authorize("blake3:application", &request)
            .expect("matching grant admits request");
        assert_eq!(grant.source.name(), "direct_development_file");
    }
}
