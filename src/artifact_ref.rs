//! Docker-style references for named `.holo` artifacts.
//!
//! `[host[:port]/]namespace/name[:tag]`, where a bare `name:tag` expands
//! against the configured default registry. Following OCI, the colon separates
//! repository from tag — `qwen3.5:4b` is repository `qwen3.5`, tag `4b` —
//! because the OCI tag grammar excludes colons.
//!
//! A reference is not a capability. Resolving one says nothing about what the
//! archive may do; see ADR 020 and the admission tests.

use crate::config::RegistryConfig;
use crate::error::{LiveError, Result};
use std::path::PathBuf;

/// A fully-qualified artifact reference, with every default already applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactRef {
    pub host: String,
    pub namespace: String,
    pub name: String,
    pub tag: String,
}

/// What an input string denotes.
///
/// The order of these variants is the resolution precedence, and that order is
/// load-bearing: `run` and `serve` already accept paths and kappas, so a path
/// must keep winning or adding reference support would silently change what an
/// existing command does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// A `blake3:` object id: look in the local catalog.
    Kappa(String),
    /// An existing filesystem path: a local archive.
    File(PathBuf),
    /// Anything else: resolve through the registry.
    Reference(ArtifactRef),
}

impl ArtifactRef {
    pub fn parse(input: &str, config: &RegistryConfig) -> Result<Self> {
        let input = input.trim();
        if input.is_empty() {
            return Err(LiveError::Protocol(
                "artifact reference is empty".to_owned(),
            ));
        }

        // Split the tag from the right, but only past the last '/': a port in
        // the host ("host:5000/app") must not be mistaken for a tag.
        let last_slash = input.rfind('/');
        let (body, tag) = match input.rfind(':') {
            Some(colon) if last_slash.is_none_or(|slash| colon > slash) => {
                (&input[..colon], &input[colon + 1..])
            }
            _ => (input, "latest"),
        };
        validate_tag(tag)?;

        let mut segments: Vec<&str> = body.split('/').filter(|s| !s.is_empty()).collect();
        // Popping and matching in one step keeps the emptiness check and the
        // use inseparable, so there is no unreachable `expect` to document.
        let Some(name) = segments.pop().map(str::to_owned) else {
            return Err(LiveError::Protocol(format!(
                "artifact reference {input:?} has no name"
            )));
        };
        validate_segment(&name, "name")?;

        // A leading segment is a host only if it looks like one. "team/app"
        // must stay namespace/name against the default host.
        let host = if segments.first().is_some_and(|first| is_host(first)) {
            segments.remove(0).to_owned()
        } else {
            default_host(config)?
        };

        let namespace = if segments.is_empty() {
            config.namespace.trim_matches('/').to_owned()
        } else {
            for segment in &segments {
                validate_segment(segment, "namespace")?;
            }
            segments.join("/")
        };
        if namespace.is_empty() {
            return Err(LiveError::Protocol(
                "artifact reference has no namespace and registry.namespace is empty".to_owned(),
            ));
        }

        Ok(Self {
            host,
            namespace,
            name,
            tag: tag.to_owned(),
        })
    }

    /// `namespace/name`, the repository portion of an upstream path.
    pub fn repository(&self) -> String {
        format!("{}/{}", self.namespace, self.name)
    }

    /// Canonical fully-qualified form, suitable for logs and audit records.
    pub fn display(&self) -> String {
        format!(
            "{}/{}/{}:{}",
            self.host, self.namespace, self.name, self.tag
        )
    }
}

/// Classify an input against the precedence rules.
pub fn resolve(input: &str, config: &RegistryConfig) -> Result<Resolution> {
    let trimmed = input.trim();
    if is_kappa(trimmed) {
        return Ok(Resolution::Kappa(trimmed.to_owned()));
    }
    let path = PathBuf::from(trimmed);
    if path.exists() {
        return Ok(Resolution::File(path));
    }
    Ok(Resolution::Reference(ArtifactRef::parse(trimmed, config)?))
}

fn is_kappa(input: &str) -> bool {
    input.strip_prefix("blake3:").is_some_and(|digest| {
        digest.len() == 64
            && digest
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    })
}

/// A leading segment is a host if it carries a port or a dot. This is the
/// docker heuristic, and it is why `team/app` stays namespace/name.
fn is_host(segment: &str) -> bool {
    segment.contains(':') || segment.contains('.') || segment == "localhost"
}

fn default_host(config: &RegistryConfig) -> Result<String> {
    let endpoint = config.endpoint.trim();
    let without_scheme = endpoint
        .strip_prefix("https://")
        .or_else(|| endpoint.strip_prefix("http://"))
        .unwrap_or(endpoint);
    let host = without_scheme.trim_end_matches('/');
    if host.is_empty() {
        return Err(LiveError::Config(
            "registry.endpoint must be set to resolve a bare artifact reference".to_owned(),
        ));
    }
    Ok(host.to_owned())
}

/// The OCI tag grammar: `[a-zA-Z0-9_][a-zA-Z0-9._-]{0,127}`.
fn validate_tag(tag: &str) -> Result<()> {
    if tag.is_empty() || tag.len() > 128 {
        return Err(LiveError::Protocol(format!(
            "artifact tag {tag:?} must be 1 to 128 characters"
        )));
    }
    let mut characters = tag.chars();
    let Some(first) = characters.next() else {
        return Err(LiveError::Protocol(format!(
            "artifact tag {tag:?} must be 1 to 128 characters"
        )));
    };
    if !(first.is_ascii_alphanumeric() || first == '_') {
        return Err(LiveError::Protocol(format!(
            "artifact tag {tag:?} must start with an alphanumeric or underscore"
        )));
    }
    if !characters.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')) {
        return Err(LiveError::Protocol(format!(
            "artifact tag {tag:?} contains a character outside the OCI tag grammar"
        )));
    }
    Ok(())
}

fn validate_segment(segment: &str, label: &str) -> Result<()> {
    if segment.is_empty() {
        return Err(LiveError::Protocol(format!(
            "artifact reference {label} segment is empty"
        )));
    }
    if !segment
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
    {
        return Err(LiveError::Protocol(format!(
            "artifact reference {label} {segment:?} must be lowercase alphanumeric with '.', '_', or '-'"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RegistryConfig;

    fn config() -> RegistryConfig {
        RegistryConfig {
            endpoint: "http://registry.example:5000".to_owned(),
            namespace: "models".to_owned(),
            ..RegistryConfig::default()
        }
    }

    #[test]
    fn a_bare_name_and_tag_expands_against_the_configured_default() {
        let reference = ArtifactRef::parse("qwen3.5:4b", &config()).expect("parse");
        assert_eq!(reference.host, "registry.example:5000");
        assert_eq!(reference.namespace, "models");
        assert_eq!(reference.name, "qwen3.5");
        assert_eq!(reference.tag, "4b");
        assert_eq!(reference.repository(), "models/qwen3.5");
    }

    #[test]
    fn an_omitted_tag_defaults_to_latest() {
        let reference = ArtifactRef::parse("qwen3.5", &config()).expect("parse");
        assert_eq!(reference.tag, "latest");
        assert_eq!(reference.name, "qwen3.5");
    }

    #[test]
    fn a_fully_qualified_reference_overrides_every_default() {
        let reference =
            ArtifactRef::parse("other.host:5001/team/app:v2", &config()).expect("parse");
        assert_eq!(reference.host, "other.host:5001");
        assert_eq!(reference.namespace, "team");
        assert_eq!(reference.name, "app");
        assert_eq!(reference.tag, "v2");
        assert_eq!(reference.display(), "other.host:5001/team/app:v2");
    }

    #[test]
    fn a_nested_namespace_keeps_every_segment() {
        let reference = ArtifactRef::parse("host:5000/team/sub/app:v1", &config()).expect("parse");
        assert_eq!(reference.namespace, "team/sub");
        assert_eq!(reference.name, "app");
        assert_eq!(reference.repository(), "team/sub/app");
    }

    #[test]
    fn a_host_port_is_not_mistaken_for_a_tag() {
        // "host:5000/app" splits on the last colon, which precedes the slash.
        // Reading that as tag "5000/app" would be silently wrong.
        let reference = ArtifactRef::parse("host:5000/app", &config()).expect("parse");
        assert_eq!(reference.host, "host:5000");
        assert_eq!(reference.name, "app");
        assert_eq!(reference.tag, "latest");
    }

    #[test]
    fn a_tag_must_satisfy_the_oci_grammar() {
        // Colons separate repository from tag, so a tag containing one is not
        // representable and must be rejected rather than silently mangled.
        assert!(ArtifactRef::parse("app:bad:tag", &config()).is_err());
        assert!(ArtifactRef::parse("app:-leading", &config()).is_err());
        assert!(ArtifactRef::parse("app:has space", &config()).is_err());
        assert!(ArtifactRef::parse("app:", &config()).is_err());
        assert!(ArtifactRef::parse("", &config()).is_err());
    }

    #[test]
    fn a_kappa_resolves_to_the_catalog_not_the_registry() {
        let input = "blake3:cb9ef1526f722fcaaf5a6e19610d045349627e4ce70ad4420ccc00b6bfc5e959";
        match resolve(input, &config()).expect("resolve") {
            Resolution::Kappa(kappa) => assert_eq!(kappa, input),
            other => panic!("expected a kappa resolution, got {other:?}"),
        }
    }

    #[test]
    fn an_existing_path_wins_over_a_registry_reference() {
        // Path-first precedence is what guarantees no existing invocation of
        // `run` changes meaning when reference support is added.
        let directory = std::env::temp_dir().join(format!(
            "hologram-ref-{}-{}",
            std::process::id(),
            crate::util::now_millis()
        ));
        std::fs::create_dir_all(&directory).expect("create");
        let file = directory.join("qwen3.5:4b");
        std::fs::write(&file, b"archive").expect("write");

        let input = file.to_string_lossy().into_owned();
        match resolve(&input, &config()).expect("resolve") {
            Resolution::File(path) => assert_eq!(path, file),
            other => panic!("an existing file must win, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn a_name_that_is_not_a_path_resolves_as_a_reference() {
        match resolve("qwen3.5:4b", &config()).expect("resolve") {
            Resolution::Reference(reference) => assert_eq!(reference.name, "qwen3.5"),
            other => panic!("expected a registry reference, got {other:?}"),
        }
    }

    #[test]
    fn a_reference_carries_no_capability_vocabulary() {
        // Pulling confers no authority. The reference type is the only thing a
        // pull adds to the execution path, so it must not be able to express
        // authority at all; this is the tripwire if a grant-shaped field is
        // ever added.
        let reference = ArtifactRef::parse("qwen3.5:4b", &config()).expect("parse");
        let rendered = format!("{reference:?}").to_lowercase();
        for forbidden in [
            "grant",
            "capability",
            "scope",
            "permission",
            "trusted",
            "privilege",
        ] {
            assert!(
                !rendered.contains(forbidden),
                "an artifact reference must not carry {forbidden}: {rendered}"
            );
        }
    }
}
