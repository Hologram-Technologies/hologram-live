//! The scenario format: a scripted session, written once and played against
//! both registries. Everything generated here is a pure function of the file,
//! so the same scenario sends the same bytes on every run and every machine.

use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;
use std::path::Path;

const OCI_MANIFEST: &str = "application/vnd.oci.image.manifest.v1+json";
const OCI_INDEX: &str = "application/vnd.oci.image.index.v1+json";
const DOCKER_LIST: &str = "application/vnd.docker.distribution.manifest.list.v2+json";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    pub name: String,
    pub summary: String,
    /// Any of `delete`, `auth`, `readonly`: the settings both sides start with.
    #[serde(default)]
    pub needs: Vec<String>,
    pub repo: String,
    #[serde(default)]
    pub manifests: BTreeMap<String, ManifestSpec>,
    #[serde(rename = "step")]
    pub steps: Vec<StepSpec>,
    #[serde(default)]
    pub state: StateSpec,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepSpec {
    pub id: String,
    pub method: String,
    /// A path under the registry, or `url`: a captured `Location`, followed verbatim.
    pub path: Option<String>,
    pub url: Option<String>,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub query: BTreeMap<String, String>,
    pub body: Option<String>,
    /// `name = "header:<name>"`: keep a response header for later steps.
    #[serde(default)]
    pub capture: BTreeMap<String, String>,
    /// What the REFERENCE answers. It catches a wrong scenario; it is never
    /// the product's test.
    pub expect_status: Option<u16>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateSpec {
    #[serde(default)]
    pub catalog: bool,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestSpec {
    #[serde(default = "oci_manifest")]
    pub media_type: String,
    /// Blob specs: `seed=<n>:len=<m>`.
    pub config: Option<String>,
    #[serde(default)]
    pub layers: Vec<String>,
    /// For an index: names of other manifests in this scenario.
    #[serde(default)]
    pub children: Vec<String>,
    /// The name of another manifest in this scenario.
    pub subject: Option<String>,
    pub artifact_type: Option<String>,
    /// Leave `mediaType` out of the body.
    #[serde(default)]
    pub omit_media_type: bool,
}

fn oci_manifest() -> String {
    OCI_MANIFEST.to_owned()
}

impl Scenario {
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let scenario: Self = toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        for step in &scenario.steps {
            if step.path.is_some() == step.url.is_some() {
                return Err(format!("{}: step {} needs exactly one of path, url", scenario.name, step.id));
            }
        }
        Ok(scenario)
    }

    /// The bytes of manifest `name`, built the same way every time.
    pub fn manifest(&self, name: &str) -> Result<Vec<u8>, String> {
        let spec = self.manifests.get(name).ok_or_else(|| format!("{}: no manifest {name}", self.name))?;
        let mut fields = vec!["\"schemaVersion\":2".to_owned()];
        if !spec.omit_media_type {
            fields.push(format!("\"mediaType\":\"{}\"", spec.media_type));
        }
        if let Some(artifact_type) = &spec.artifact_type {
            fields.push(format!("\"artifactType\":\"{artifact_type}\""));
        }
        if spec.media_type == OCI_INDEX || spec.media_type == DOCKER_LIST {
            let mut children = Vec::new();
            for child in &spec.children {
                let bytes = self.manifest(child)?;
                let media_type = &self.manifests[child].media_type;
                children.push(descriptor(media_type, &bytes));
            }
            fields.push(format!("\"manifests\":[{}]", children.join(",")));
        } else {
            let docker = spec.media_type.contains("docker");
            if let Some(config) = &spec.config {
                let media_type = if docker {
                    "application/vnd.docker.container.image.v1+json"
                } else {
                    "application/vnd.oci.image.config.v1+json"
                };
                fields.push(format!("\"config\":{}", descriptor(media_type, &blob(config)?)));
            }
            let media_type = if docker {
                "application/vnd.docker.image.rootfs.diff.tar.gzip"
            } else {
                "application/vnd.oci.image.layer.v1.tar+gzip"
            };
            let mut layers = Vec::new();
            for layer in &spec.layers {
                layers.push(descriptor(media_type, &blob(layer)?));
            }
            fields.push(format!("\"layers\":[{}]", layers.join(",")));
        }
        if let Some(subject) = &spec.subject {
            let bytes = self.manifest(subject)?;
            fields.push(format!("\"subject\":{}", descriptor(&self.manifests[subject].media_type, &bytes)));
        }
        Ok(format!("{{{}}}", fields.join(",")).into_bytes())
    }

    /// Replace `{placeholders}`. A brace that does not hold a placeholder
    /// (JSON, mostly) is left alone.
    pub fn expand(&self, text: &str, captured: &BTreeMap<String, String>) -> Result<String, String> {
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        while let Some(open) = rest.find('{') {
            out.push_str(&rest[..open]);
            let after = &rest[open + 1..];
            let close = after.find('}');
            let inner = close.map(|close| &after[..close]).filter(|inner| {
                !inner.is_empty() && inner.chars().all(|c| c.is_ascii_alphanumeric() || "_:=./-".contains(c))
            });
            match (inner, close) {
                (Some(inner), Some(close)) => {
                    out.push_str(&self.placeholder(inner, captured)?);
                    rest = &after[close + 1..];
                }
                _ => {
                    out.push('{');
                    rest = after;
                }
            }
        }
        out.push_str(rest);
        Ok(out)
    }

    fn placeholder(&self, inner: &str, captured: &BTreeMap<String, String>) -> Result<String, String> {
        if inner == "repo" {
            return Ok(self.repo.clone());
        }
        if let Some(value) = captured.get(inner) {
            return Ok(value.clone());
        }
        if let Some(spec) = inner.strip_prefix("sha256:") {
            return Ok(sha256(&blob(spec)?));
        }
        if let Some(spec) = inner.strip_prefix("sha256upper:") {
            let digest = sha256(&blob(spec)?);
            return Ok(format!("sha256:{}", digest["sha256:".len()..].to_ascii_uppercase()));
        }
        if let Some(spec) = inner.strip_prefix("sha512:") {
            let mut out = String::from("sha512:");
            for byte in sha2::Sha512::digest(blob(spec)?) {
                out.push_str(&format!("{byte:02x}"));
            }
            return Ok(out);
        }
        if let Some(spec) = inner.strip_prefix("blake3:") {
            return Ok(format!("blake3:{}", blake3::hash(&blob(spec)?).to_hex()));
        }
        if let Some(name) = inner.strip_prefix("manifest:") {
            return Ok(sha256(&self.manifest(name)?));
        }
        if let Some(spec) = inner.strip_prefix("repeat:") {
            let (unit, count) = spec.split_once(':').ok_or_else(|| format!("repeat needs unit:count in {inner}"))?;
            let count: usize = count.parse().map_err(|_| format!("bad count in {inner}"))?;
            return Ok(unit.repeat(count));
        }
        Err(format!("{}: unknown placeholder {{{inner}}} (captured too late?)", self.name))
    }

    /// A step's body: `bytes:<blob spec>`, `manifest:<name>`, `text:<inline>`.
    pub fn body(&self, spec: &str, captured: &BTreeMap<String, String>) -> Result<Vec<u8>, String> {
        if let Some(spec) = spec.strip_prefix("bytes:") {
            return blob(spec);
        }
        if let Some(name) = spec.strip_prefix("manifest:") {
            return self.manifest(name);
        }
        if let Some(text) = spec.strip_prefix("text:") {
            return Ok(self.expand(text, captured)?.into_bytes());
        }
        Err(format!("{}: unknown body generator {spec:?}", self.name))
    }
}

fn descriptor(media_type: &str, bytes: &[u8]) -> String {
    format!("{{\"mediaType\":\"{media_type}\",\"digest\":\"{}\",\"size\":{}}}", sha256(bytes), bytes.len())
}

pub fn sha256(bytes: &[u8]) -> String {
    let mut out = String::from("sha256:");
    for byte in Sha256::digest(bytes) {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// `seed=<n>:len=<m>`: bytes from a blake3 stream keyed by the seed alone.
pub fn blob(spec: &str) -> Result<Vec<u8>, String> {
    let mut seed = None;
    let mut len = None;
    for part in spec.split(':') {
        match part.split_once('=') {
            Some(("seed", value)) => seed = value.parse::<u64>().ok(),
            Some(("len", value)) => len = value.parse::<usize>().ok(),
            _ => return Err(format!("bad blob spec {spec:?}")),
        }
    }
    let (Some(seed), Some(len)) = (seed, len) else {
        return Err(format!("blob spec {spec:?} needs seed and len"));
    };
    let mut out = vec![0_u8; len];
    blake3::Hasher::new().update(&seed.to_le_bytes()).finalize_xof().fill(&mut out);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scenario() -> Scenario {
        toml::from_str(
            r#"
name = "t"
summary = "s"
repo = "gate-b/t"
[manifests.image]
config = "seed=1:len=8"
layers = ["seed=2:len=16"]
[manifests.list]
media_type = "application/vnd.oci.image.index.v1+json"
children = ["image"]
[[step]]
id = "a"
method = "GET"
path = "/v2/"
"#,
        )
        .expect("scenario")
    }

    #[test]
    fn generated_bytes_depend_on_the_seed_alone() {
        assert_eq!(blob("seed=7:len=32").expect("blob"), blob("seed=7:len=32").expect("blob"));
        assert_ne!(blob("seed=7:len=32").expect("blob"), blob("seed=8:len=32").expect("blob"));
        assert!(blob("len=3").is_err());
    }

    #[test]
    fn placeholders_expand_and_json_braces_do_not() {
        let scenario = scenario();
        let mut captured = BTreeMap::new();
        captured.insert("upload".to_owned(), "/v2/x/blobs/uploads/1".to_owned());
        let text = scenario
            .expand(r#"{repo} {upload} {"a": {"b": 1}} {repeat:ab:3} {sha256:seed=1:len=8}"#, &captured)
            .expect("expand");
        assert!(text.starts_with(r#"gate-b/t /v2/x/blobs/uploads/1 {"a": {"b": 1}} ababab sha256:"#), "{text}");
        assert!(scenario.expand("{nothing}", &captured).is_err());
    }

    #[test]
    fn an_index_names_its_child_by_the_childs_digest() {
        let scenario = scenario();
        let image = scenario.manifest("image").expect("image");
        let list = String::from_utf8(scenario.manifest("list").expect("list")).expect("utf8");
        assert!(list.contains(&sha256(&image)));
        assert!(list.contains(&format!("\"size\":{}", image.len())));
    }
}
