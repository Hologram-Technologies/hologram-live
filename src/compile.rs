use crate::error::{LiveError, Result};
use crate::holo_directory::{self, DIRECTORY_EXTENSION_KEY};
use crate::holo_python::{self, PythonRootfsSource};
use crate::protocol::HoloIdentity;
use crate::util::hex;
use hologram::archive::{HoloLoader, HoloWriter};
use hologram::space::{address_bytes, AppManifest, Layer, Realization};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const CURRENT_MANIFEST_SCHEMA_VERSION: u16 = 2;

/// Source manifest accepted by `hologram compile`.
///
/// Paths are resolved relative to the manifest file. The resulting archive is
/// a self-contained Hologram v4 application: every layer and the declared
/// capability set are embedded under their canonical kappa labels.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompileManifest {
    #[serde(default = "default_schema_version")]
    pub schema_version: u16,
    #[serde(default)]
    pub primary: Option<u32>,
    #[serde(default)]
    pub requires: Option<PathBuf>,
    pub layers: Vec<CompileLayer>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompileLayer {
    pub kind: CompileLayerKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<CompileSource>,
    #[serde(default)]
    pub entry: Option<String>,
    #[serde(default)]
    pub arch: Option<String>,
    #[serde(default)]
    pub surface: Option<String>,
    #[serde(default)]
    pub engine: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CompileLayerKind {
    Wasm,
    Tensor,
    Rootfs,
    View,
    InferenceModel,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "language", rename_all = "kebab-case", deny_unknown_fields)]
pub enum CompileSource {
    Python(PythonRootfsSource),
}

pub struct CompiledHolo {
    pub bytes: Vec<u8>,
    pub layer_count: usize,
    pub packaging: HoloPackaging,
    pub identity: HoloIdentity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoloPackaging {
    Fat,
    Thin,
}

pub fn compile_manifest(path: &Path) -> Result<CompiledHolo> {
    compile_manifest_with(path, HoloPackaging::Fat)
}

pub fn check_manifest(path: &Path) -> Result<CompileManifest> {
    let source = std::fs::read(path).map_err(|error| LiveError::io(path, error))?;
    let specification = parse_compile_manifest(path, &source)?;
    validate_compile_manifest(&specification)?;
    let root = path.parent().unwrap_or_else(|| Path::new("."));
    let _ = read_relative(root, specification.requires.as_deref())?;
    for layer in &specification.layers {
        match (&layer.path, &layer.source) {
            (Some(path), None) => {
                let _ = read_required(root, path)?;
            }
            (None, Some(CompileSource::Python(source))) => {
                let arch = required_field(layer, "arch", layer.arch.as_deref())?;
                holo_python::check_source(root, source, arch)?;
            }
            _ => {
                return Err(layer_config_error(
                    layer,
                    "exactly one of path or source is required",
                ));
            }
        }
    }
    Ok(specification)
}

pub fn compile_manifest_with(path: &Path, packaging: HoloPackaging) -> Result<CompiledHolo> {
    let source = std::fs::read(path).map_err(|error| LiveError::io(path, error))?;
    let specification = parse_compile_manifest(path, &source)?;
    validate_compile_manifest(&specification)?;

    let root = path.parent().unwrap_or_else(|| Path::new("."));
    let requires_bytes = read_relative(root, specification.requires.as_deref())?;
    let requires = address_bytes(&requires_bytes);
    let mut blobs = BTreeMap::new();
    blobs.insert(requires.as_bytes().to_vec(), requires_bytes);

    let mut layers = Vec::with_capacity(specification.layers.len());
    for source_layer in &specification.layers {
        let content = compile_layer_content(root, source_layer)?;
        let kappa = address_bytes(&content);
        blobs.insert(kappa.as_bytes().to_vec(), content);
        layers.push(build_layer(source_layer, kappa)?);
    }

    let manifest = AppManifest {
        primary: specification.primary,
        requires,
        layers,
        children: Vec::new(),
    };
    manifest.validate().map_err(|error| {
        LiveError::InvalidHolo(format!("invalid application manifest: {error:?}"))
    })?;

    let layer_count = manifest.layers.len();
    let manifest_bytes = manifest.canonicalize();
    let application_kappa = address_bytes(&manifest_bytes).to_string();
    let embedded = match packaging {
        HoloPackaging::Fat => blobs
            .iter()
            .map(|(kappa, content)| (kappa.as_slice(), content.as_slice()))
            .collect::<Vec<_>>(),
        HoloPackaging::Thin => Vec::new(),
    };
    let directory = holo_directory::derive(&manifest, embedded.iter().copied())?;
    let mut writer = HoloWriter::new();
    writer.set_app_manifest(manifest_bytes);
    writer.set_metadata(source);
    writer.add_extension(DIRECTORY_EXTENSION_KEY, holo_directory::encode(&directory)?);
    if packaging == HoloPackaging::Fat {
        for (kappa, content) in blobs {
            writer.add_content_blob(kappa, content);
        }
    }
    let bytes = writer
        .finish()
        .map_err(|error| LiveError::InvalidHolo(error.to_string()))?;
    let archive = HoloLoader::from_bytes(&bytes)
        .map_err(|error| LiveError::InvalidHolo(error.to_string()))?;
    let identity = HoloIdentity {
        archive_kappa: address_bytes(&bytes).to_string(),
        archive_fingerprint: hex(&archive.fingerprint()),
        application_kappa,
    };
    Ok(CompiledHolo {
        bytes,
        layer_count,
        packaging,
        identity,
    })
}

pub fn parse_compile_manifest(path: &Path, source: &[u8]) -> Result<CompileManifest> {
    serde_json::from_slice(source).map_err(|error| {
        LiveError::Config(format!(
            "parse compile manifest {}: {error}",
            path.display()
        ))
    })
}

pub fn validate_compile_manifest(specification: &CompileManifest) -> Result<()> {
    if !matches!(
        specification.schema_version,
        1 | CURRENT_MANIFEST_SCHEMA_VERSION
    ) {
        return Err(LiveError::Config(format!(
            "unsupported compile manifest schema {}; expected 1 or {CURRENT_MANIFEST_SCHEMA_VERSION}",
            specification.schema_version
        )));
    }
    let mut layers = Vec::with_capacity(specification.layers.len());
    for source_layer in &specification.layers {
        validate_layer_source(specification.schema_version, source_layer)?;
        layers.push(build_layer(source_layer, address_bytes(&[]))?);
    }
    let manifest = AppManifest {
        primary: specification.primary,
        requires: address_bytes(&[]),
        layers,
        children: Vec::new(),
    };
    manifest
        .validate()
        .map_err(|error| LiveError::Config(format!("invalid application manifest: {error:?}")))?;
    Ok(())
}

fn build_layer(source: &CompileLayer, kappa: hologram::space::KappaLabel71) -> Result<Layer> {
    match source.kind {
        CompileLayerKind::Wasm => {
            reject_aux(source, "arch", source.arch.as_deref())?;
            reject_aux(source, "surface", source.surface.as_deref())?;
            reject_aux(source, "engine", source.engine.as_deref())?;
            Ok(Layer::wasm(
                kappa,
                effective_entry(source).unwrap_or("_start"),
            ))
        }
        CompileLayerKind::Tensor => {
            reject_aux(source, "arch", source.arch.as_deref())?;
            reject_aux(source, "surface", source.surface.as_deref())?;
            reject_aux(source, "engine", source.engine.as_deref())?;
            Ok(Layer::tensor(
                kappa,
                effective_entry(source).unwrap_or("session"),
            ))
        }
        CompileLayerKind::Rootfs => {
            reject_aux(source, "surface", source.surface.as_deref())?;
            reject_aux(source, "engine", source.engine.as_deref())?;
            let arch = required_field(source, "arch", source.arch.as_deref())?;
            let arch = if matches!(source.source.as_ref(), Some(CompileSource::Python(_))) {
                holo_python::canonical_arch(arch)?
            } else {
                arch
            };
            Ok(Layer::rootfs(
                kappa,
                effective_entry(source).unwrap_or("boot"),
                arch,
            ))
        }
        CompileLayerKind::View => {
            reject_aux(source, "arch", source.arch.as_deref())?;
            reject_aux(source, "engine", source.engine.as_deref())?;
            if source.entry.is_some() {
                return Err(layer_config_error(
                    source,
                    "view layers do not accept an entry field",
                ));
            }
            let surface = required_field(source, "surface", source.surface.as_deref())?;
            Ok(Layer::view(kappa, surface))
        }
        CompileLayerKind::InferenceModel => {
            reject_aux(source, "arch", source.arch.as_deref())?;
            reject_aux(source, "surface", source.surface.as_deref())?;
            let entry = required_field(source, "entry", source.entry.as_deref())?;
            let engine = required_field(source, "engine", source.engine.as_deref())?;
            Ok(Layer::inference_model(kappa, entry, engine))
        }
    }
}

fn read_relative(root: &Path, path: Option<&Path>) -> Result<Vec<u8>> {
    path.map_or_else(|| Ok(Vec::new()), |path| read_required(root, path))
}

fn read_required(root: &Path, path: &Path) -> Result<Vec<u8>> {
    let resolved = root.join(path);
    std::fs::read(&resolved).map_err(|error| LiveError::io(&resolved, error))
}

fn compile_layer_content(root: &Path, layer: &CompileLayer) -> Result<Vec<u8>> {
    match (&layer.path, &layer.source) {
        (Some(path), None) => read_required(root, path),
        (None, Some(CompileSource::Python(source))) => {
            let arch = required_field(layer, "arch", layer.arch.as_deref())?;
            holo_python::compile(root, source, arch)
        }
        _ => Err(layer_config_error(
            layer,
            "exactly one of path or source is required",
        )),
    }
}

fn validate_layer_source(schema_version: u16, layer: &CompileLayer) -> Result<()> {
    match (&layer.path, &layer.source) {
        (Some(path), None) if !path.as_os_str().is_empty() => Ok(()),
        (None, Some(_)) if schema_version == 1 => Err(layer_config_error(
            layer,
            "source recipes require schema_version 2",
        )),
        (None, Some(CompileSource::Python(source))) => {
            if !matches!(layer.kind, CompileLayerKind::Rootfs) {
                return Err(layer_config_error(
                    layer,
                    "Python rootfs sources require kind rootfs",
                ));
            }
            if layer.entry.is_some() {
                return Err(layer_config_error(
                    layer,
                    "Python source entry belongs inside source",
                ));
            }
            holo_python::validate_source(source, layer.arch.as_deref())
        }
        _ => Err(layer_config_error(
            layer,
            "exactly one of path or source is required",
        )),
    }
}

fn effective_entry(layer: &CompileLayer) -> Option<&str> {
    layer.entry.as_deref().or(match layer.source.as_ref() {
        Some(CompileSource::Python(source)) => Some(source.entry.as_str()),
        None => None,
    })
}

fn required_field<'a>(
    layer: &CompileLayer,
    field: &str,
    value: Option<&'a str>,
) -> Result<&'a str> {
    value.filter(|value| !value.is_empty()).ok_or_else(|| {
        layer_config_error(layer, &format!("{} layers require {field}", kind(layer)))
    })
}

fn reject_aux(layer: &CompileLayer, field: &str, value: Option<&str>) -> Result<()> {
    if value.is_some() {
        return Err(layer_config_error(
            layer,
            &format!("{} layers do not accept {field}", kind(layer)),
        ));
    }
    Ok(())
}

fn layer_config_error(layer: &CompileLayer, message: &str) -> LiveError {
    LiveError::Config(format!(
        "compile layer {} ({}): {message}",
        layer_name(layer),
        kind(layer)
    ))
}

fn layer_name(layer: &CompileLayer) -> String {
    layer.path.as_ref().map_or_else(
        || match layer.source.as_ref() {
            Some(CompileSource::Python(source)) => source.project.display().to_string(),
            None => "<missing>".to_owned(),
        },
        |path| path.display().to_string(),
    )
}

const fn kind(layer: &CompileLayer) -> &'static str {
    match layer.kind {
        CompileLayerKind::Wasm => "wasm",
        CompileLayerKind::Tensor => "tensor",
        CompileLayerKind::Rootfs => "rootfs",
        CompileLayerKind::View => "view",
        CompileLayerKind::InferenceModel => "inference-model",
    }
}

const fn default_schema_version() -> u16 {
    1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::holo::inspect_bytes;

    #[test]
    fn compiles_a_self_contained_view_application() {
        let directory = tempfile::tempdir().expect("tempdir");
        std::fs::write(directory.path().join("view.html"), "<h1>Hello</h1>").expect("view");
        std::fs::write(
            directory.path().join("hologram.json"),
            r#"{
                "schema_version": 1,
                "layers": [{"kind":"view","path":"view.html","surface":"portable"}]
            }"#,
        )
        .expect("manifest");

        let compiled = compile_manifest(&directory.path().join("hologram.json")).expect("compile");
        assert_eq!(compiled.layer_count, 1);
        let inspection = inspect_bytes(
            &compiled.identity.archive_kappa,
            "hello.holo",
            &compiled.bytes,
        )
        .expect("inspect");
        assert_eq!(inspection.kappa, compiled.identity.archive_kappa);
        assert_eq!(
            inspection.application_kappa.as_deref(),
            Some(compiled.identity.application_kappa.as_str())
        );
        assert_eq!(
            inspection.archive_fingerprint,
            compiled.identity.archive_fingerprint
        );
        assert!(inspection
            .sections
            .iter()
            .any(|section| section.kind == "AppManifest"));
        assert!(inspection
            .sections
            .iter()
            .any(|section| section.kind == "ContentBlob"));
        assert!(inspection.directory_embedded);
        let directory = inspection.directory.expect("application directory");
        assert_eq!(directory.schema_version, 1);
        assert_eq!(directory.layers.len(), 1);
        assert_eq!(directory.layers[0].kind, "view");
        assert_eq!(directory.layers[0].surface.as_deref(), Some("portable"));
        assert_eq!(directory.blobs.len(), 2);
    }

    #[test]
    fn fat_and_thin_packages_share_the_same_application_manifest() {
        let directory = tempfile::tempdir().expect("tempdir");
        std::fs::write(directory.path().join("view.html"), "<h1>Hello</h1>").expect("view");
        let manifest_path = directory.path().join("hologram.json");
        std::fs::write(
            &manifest_path,
            r#"{
                "schema_version": 1,
                "layers": [{"kind":"view","path":"view.html","surface":"portable"}]
            }"#,
        )
        .expect("manifest");

        let fat = compile_manifest_with(&manifest_path, HoloPackaging::Fat).expect("fat");
        let thin = compile_manifest_with(&manifest_path, HoloPackaging::Thin).expect("thin");
        assert_eq!(fat.packaging, HoloPackaging::Fat);
        assert_eq!(thin.packaging, HoloPackaging::Thin);

        let fat_loader = hologram::archive::HoloLoader::from_bytes(&fat.bytes).expect("fat loader");
        let thin_loader =
            hologram::archive::HoloLoader::from_bytes(&thin.bytes).expect("thin loader");
        let fat_plan = fat_loader.into_plan().expect("fat plan");
        let thin_plan = thin_loader.into_plan().expect("thin plan");
        assert_eq!(fat_plan.app_manifest(), thin_plan.app_manifest());
        assert_eq!(fat_plan.content_blobs().expect("fat blobs").len(), 2);
        assert!(thin_plan.content_blobs().expect("thin blobs").is_empty());

        assert_ne!(fat.identity.archive_kappa, thin.identity.archive_kappa);
        assert_ne!(
            fat.identity.archive_fingerprint,
            thin.identity.archive_fingerprint
        );
        assert_eq!(
            fat.identity.application_kappa,
            thin.identity.application_kappa
        );

        let fat_inspection = inspect_bytes(&fat.identity.archive_kappa, "hello.holo", &fat.bytes)
            .expect("inspect fat");
        let thin_inspection =
            inspect_bytes(&thin.identity.archive_kappa, "hello.thin.holo", &thin.bytes)
                .expect("inspect thin");
        assert_ne!(fat_inspection.kappa, thin_inspection.kappa);
        assert_ne!(
            fat_inspection.archive_fingerprint,
            thin_inspection.archive_fingerprint
        );
        assert_eq!(
            fat_inspection.application_kappa,
            thin_inspection.application_kappa
        );
        assert!(thin_inspection.directory_embedded);
        assert!(thin_inspection
            .directory
            .expect("directory")
            .blobs
            .is_empty());
    }

    #[test]
    fn schema_two_accepts_a_python_rootfs_source() {
        let manifest: CompileManifest = serde_json::from_str(
            r#"{
                "schema_version": 2,
                "primary": 0,
                "layers": [{
                    "kind": "rootfs",
                    "source": {
                        "language": "python",
                        "project": ".",
                        "entry": "analytics:main",
                        "lock": "uv.lock",
                        "profile": "rootfs",
                        "base": "python:3.12-slim"
                    },
                    "arch": "arm64"
                }]
            }"#,
        )
        .expect("parse");
        validate_compile_manifest(&manifest).expect("validate");
        assert!(manifest.layers[0].path.is_none());
        assert!(matches!(
            manifest.layers[0].source,
            Some(CompileSource::Python(_))
        ));
    }

    #[test]
    fn schema_one_rejects_source_recipes() {
        let manifest: CompileManifest = serde_json::from_str(
            r#"{
                "schema_version": 1,
                "primary": 0,
                "layers": [{
                    "kind": "rootfs",
                    "source": {
                        "language": "python",
                        "project": ".",
                        "entry": "analytics:main",
                        "lock": "uv.lock",
                        "profile": "rootfs"
                    },
                    "arch": "arm64"
                }]
            }"#,
        )
        .expect("parse");
        let error = validate_compile_manifest(&manifest).expect_err("schema mismatch");
        assert!(error.to_string().contains("schema_version 2"), "{error}");
    }

    #[test]
    fn packages_a_precompiled_inference_bundle_as_a_v4_model_layer() {
        let directory = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            directory.path().join("model.bundle"),
            b"deterministic R4 bundle",
        )
        .expect("bundle");
        let manifest_path = directory.path().join("hologram.json");
        std::fs::write(
            &manifest_path,
            r#"{
                "schema_version": 1,
                "layers": [{
                    "kind": "inference-model",
                    "path": "model.bundle",
                    "entry": "ai.default",
                    "engine": "uor-r4"
                }]
            }"#,
        )
        .expect("manifest");

        let compiled = compile_manifest(&manifest_path).expect("compile model archive");
        let inspection = inspect_bytes("model", "model.holo", &compiled.bytes).expect("inspect");
        assert_eq!(inspection.format_version, 4);
        let directory = inspection.directory.expect("application directory");
        assert_eq!(directory.primary_layer, None);
        assert_eq!(directory.layers[0].kind, "inference-model");
        assert_eq!(directory.layers[0].entry, "ai.default");
        assert_eq!(directory.layers[0].engine.as_deref(), Some("uor-r4"));
        assert_eq!(directory.blobs.len(), 2);
    }

    #[test]
    fn inference_model_layers_require_an_entry_and_engine() {
        let missing_engine: CompileManifest = serde_json::from_str(
            r#"{
                "layers": [{
                    "kind": "inference-model",
                    "path": "model.bundle",
                    "entry": "ai.default"
                }]
            }"#,
        )
        .expect("parse");
        let error = validate_compile_manifest(&missing_engine).expect_err("engine is required");
        assert!(error.to_string().contains("require engine"), "{error}");

        let missing_entry: CompileManifest = serde_json::from_str(
            r#"{
                "layers": [{
                    "kind": "inference-model",
                    "path": "model.bundle",
                    "engine": "uor-r4"
                }]
            }"#,
        )
        .expect("parse");
        let error = validate_compile_manifest(&missing_entry).expect_err("entry is required");
        assert!(error.to_string().contains("require entry"), "{error}");
    }
}
