use crate::application_plan::HoloIdentity;
use crate::error::{LiveError, Result};
use crate::holo_capability;
use crate::holo_contract::{
    normalize_wasm_contract, COMPONENT_V1_ENTRY, WASM_CONTRACT_COMPONENT_CHANNEL_PUBLISH_V1,
    WASM_CONTRACT_COMPONENT_CHANNEL_SUBSCRIBE_V1, WASM_CONTRACT_COMPONENT_NETWORK_FETCH_V1,
    WASM_CONTRACT_COMPONENT_STORE_GRAPH_READ_V1, WASM_CONTRACT_COMPONENT_STORE_READ_V1,
    WASM_CONTRACT_COMPONENT_STORE_WRITE_V1, WASM_CONTRACT_COMPONENT_V1,
};
use crate::holo_directory::{self, DIRECTORY_EXTENSION_KEY};
use crate::holo_python::{self, PythonRootfsSource};
use crate::holo_python_component;
use crate::holo_view;
use crate::holo_wasm::validate_entry_name;
use crate::util::hex;
use hologram::archive::{HoloLoader, HoloWriter};
use hologram::space::{address_bytes, AppManifest, KappaLabel71, Layer, Realization};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const CURRENT_MANIFEST_SCHEMA_VERSION: u16 = 4;

/// Source manifest accepted by `hologram compile`.
///
/// Paths are resolved relative to the manifest file. The resulting archive is
/// a self-contained Hologram v4 application: every layer and the declared
/// capability set are embedded under their canonical kappa labels.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompileManifest {
    pub schema_version: u16,
    #[serde(default)]
    pub primary: Option<u32>,
    #[serde(default)]
    pub requires: Option<PathBuf>,
    pub layers: Vec<CompileLayer>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<CompileChild>,
}

/// One child application composed by its canonical manifest identity.
///
/// The source points at a self-contained child archive so a fat parent can
/// carry the child's verified manifest and content closure. `capabilities` is
/// a source-schema capability document compiled into the delegated set named
/// by the parent manifest edge.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CompileChild {
    pub application: PathBuf,
    pub capabilities: PathBuf,
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
    /// Canonical guest-contract selector for Wasm layers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contract: Option<String>,
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
    pub child_count: usize,
    pub packaging: HoloPackaging,
    pub identity: HoloIdentity,
    pub capabilities_kappa: String,
    pub build_provenance: BuildProvenanceReport,
}

#[derive(Debug)]
pub struct CheckedManifest {
    pub specification: CompileManifest,
    pub capabilities_kappa: String,
    pub build_provenance: BuildProvenanceReport,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BuildProvenanceReport {
    pub schema_version: u16,
    pub canonical: bool,
    pub layers: Vec<LayerBuildProvenance>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LayerBuildProvenance {
    pub layer_index: usize,
    pub language: &'static str,
    pub source: PythonBuildProvenance,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub enum PythonBuildProvenance {
    Rootfs(Box<holo_python::BuildProvenance>),
    Component(Box<holo_python_component::BuildProvenance>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoloPackaging {
    Fat,
    Thin,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CompileOptions {
    /// Ask source builders to ignore reusable build caches.
    pub no_build_cache: bool,
}

pub fn compile_manifest(path: &Path) -> Result<CompiledHolo> {
    compile_manifest_with(path, HoloPackaging::Fat)
}

pub fn check_manifest(path: &Path) -> Result<CheckedManifest> {
    let source = std::fs::read(path).map_err(|error| LiveError::io(path, error))?;
    let specification = parse_compile_manifest(path, &source)?;
    validate_compile_manifest(&specification)?;
    let root = path.parent().unwrap_or_else(|| Path::new("."));
    let capabilities = compile_capabilities(root, specification.requires.as_deref())?;
    let mut provenance = Vec::new();
    for (layer_index, layer) in specification.layers.iter().enumerate() {
        match (&layer.path, &layer.source) {
            (Some(path), None) => {
                if matches!(layer.kind, CompileLayerKind::View) {
                    let _ = holo_view::compile_directory(&root.join(path))?;
                } else {
                    let _ = read_required(root, path)?;
                }
            }
            (None, Some(CompileSource::Python(source))) => match source.profile {
                holo_python::PythonProfile::Rootfs => {
                    let arch = required_field(layer, "arch", layer.arch.as_deref())?;
                    provenance.push(LayerBuildProvenance {
                        layer_index,
                        language: "python",
                        source: PythonBuildProvenance::Rootfs(Box::new(holo_python::check_source(
                            root, source, arch,
                        )?)),
                    });
                }
                holo_python::PythonProfile::WasiComponent => {
                    provenance.push(LayerBuildProvenance {
                        layer_index,
                        language: "python",
                        source: PythonBuildProvenance::Component(Box::new(
                            holo_python_component::check_source(root, source)?,
                        )),
                    });
                }
            },
            _ => {
                return Err(layer_config_error(
                    layer,
                    "exactly one of path or source is required",
                ));
            }
        }
    }
    for child in &specification.children {
        let _ = compile_child(root, child)?;
    }
    Ok(CheckedManifest {
        specification,
        capabilities_kappa: address_bytes(&capabilities).to_string(),
        build_provenance: BuildProvenanceReport {
            schema_version: 1,
            canonical: false,
            layers: provenance,
        },
    })
}

pub fn compile_manifest_with(path: &Path, packaging: HoloPackaging) -> Result<CompiledHolo> {
    compile_manifest_with_options(path, packaging, CompileOptions::default())
}

pub fn compile_manifest_with_options(
    path: &Path,
    packaging: HoloPackaging,
    options: CompileOptions,
) -> Result<CompiledHolo> {
    let source = std::fs::read(path).map_err(|error| LiveError::io(path, error))?;
    let specification = parse_compile_manifest(path, &source)?;
    validate_compile_manifest(&specification)?;

    let root = path.parent().unwrap_or_else(|| Path::new("."));
    let requires_bytes = compile_capabilities(root, specification.requires.as_deref())?;
    let requires = address_bytes(&requires_bytes);
    let mut blobs = BTreeMap::new();
    blobs.insert(requires.as_bytes().to_vec(), requires_bytes);

    let mut layers = Vec::with_capacity(specification.layers.len());
    let mut provenance = Vec::new();
    for (layer_index, source_layer) in specification.layers.iter().enumerate() {
        let compiled_layer = compile_layer_content(root, source_layer, options)?;
        if let Some(source) = compiled_layer.provenance {
            provenance.push(LayerBuildProvenance {
                layer_index,
                language: "python",
                source,
            });
        }
        let content = compiled_layer.bytes;
        let kappa = address_bytes(&content);
        blobs.insert(kappa.as_bytes().to_vec(), content);
        layers.push(build_layer(source_layer, kappa)?);
    }

    let mut children = Vec::with_capacity(specification.children.len());
    for source_child in &specification.children {
        let child = compile_child(root, source_child)?;
        for (kappa, content) in child.blobs {
            blobs.insert(kappa, content);
        }
        children.push((child.application, child.capabilities));
    }

    let manifest = AppManifest {
        primary: specification.primary,
        requires,
        layers,
        children,
    };
    manifest.validate().map_err(|error| {
        LiveError::InvalidHolo(format!("invalid application manifest: {error:?}"))
    })?;

    let layer_count = manifest.layers.len();
    let child_count = manifest.children.len();
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
        child_count,
        packaging,
        identity,
        capabilities_kappa: requires.to_string(),
        build_provenance: BuildProvenanceReport {
            schema_version: 1,
            canonical: false,
            layers: provenance,
        },
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
    if specification.schema_version != CURRENT_MANIFEST_SCHEMA_VERSION {
        return Err(LiveError::Config(format!(
            "unsupported compile manifest schema {}; expected {CURRENT_MANIFEST_SCHEMA_VERSION}",
            specification.schema_version
        )));
    }
    let mut layers = Vec::with_capacity(specification.layers.len());
    for source_layer in &specification.layers {
        validate_layer_source(source_layer)?;
        layers.push(build_layer(source_layer, address_bytes(&[]))?);
    }
    let manifest = AppManifest {
        primary: specification.primary,
        requires: address_bytes(&[]),
        layers,
        children: specification
            .children
            .iter()
            .map(|child| {
                validate_child_source(child)?;
                Ok((address_bytes(&[]), address_bytes(&[])))
            })
            .collect::<Result<Vec<_>>>()?,
    };
    manifest
        .validate()
        .map_err(|error| LiveError::Config(format!("invalid application manifest: {error:?}")))?;
    Ok(())
}

struct CompiledChild {
    application: KappaLabel71,
    capabilities: KappaLabel71,
    blobs: BTreeMap<Vec<u8>, Vec<u8>>,
}

fn compile_child(root: &Path, child: &CompileChild) -> Result<CompiledChild> {
    validate_child_source(child)?;
    let archive_path = root.join(&child.application);
    let archive_bytes =
        std::fs::read(&archive_path).map_err(|error| LiveError::io(&archive_path, error))?;
    crate::holo_format::require_current(&archive_bytes)?;
    let loader = HoloLoader::from_bytes(&archive_bytes).map_err(|error| {
        LiveError::InvalidHolo(format!(
            "read child archive {}: {error}",
            archive_path.display()
        ))
    })?;
    let plan = loader.into_plan().map_err(|error| {
        LiveError::InvalidHolo(format!(
            "read child archive {}: {error}",
            archive_path.display()
        ))
    })?;
    let manifest_bytes = plan
        .app_manifest()
        .ok_or_else(|| {
            LiveError::InvalidHolo(format!(
                "child archive {} has no application manifest",
                archive_path.display()
            ))
        })?
        .to_vec();
    let manifest = AppManifest::decode(&manifest_bytes).map_err(|error| {
        LiveError::InvalidHolo(format!(
            "decode child application manifest {}: {error:?}",
            archive_path.display()
        ))
    })?;
    manifest.validate().map_err(|error| {
        LiveError::InvalidHolo(format!(
            "invalid child application manifest {}: {error:?}",
            archive_path.display()
        ))
    })?;
    let canonical = manifest.canonicalize();
    if canonical != manifest_bytes {
        return Err(LiveError::InvalidHolo(format!(
            "child archive {} contains a non-canonical application manifest",
            archive_path.display()
        )));
    }

    let extensions = plan
        .extensions()
        .map_err(|error| LiveError::InvalidHolo(error.to_string()))?;
    let directories = extensions
        .iter()
        .filter(|(key, _)| *key == DIRECTORY_EXTENSION_KEY)
        .map(|(_, bytes)| *bytes);
    let content_blobs = plan
        .content_blobs()
        .map_err(|error| LiveError::InvalidHolo(error.to_string()))?;
    holo_directory::verify_required(&manifest, directories, content_blobs.iter().copied())?;

    let application = address_bytes(&canonical);
    let mut blobs = BTreeMap::new();
    blobs.insert(application.as_bytes().to_vec(), canonical.clone());
    for (label, content) in content_blobs {
        let actual = address_bytes(content);
        if actual.as_bytes() != label {
            return Err(LiveError::InvalidHolo(format!(
                "child archive {} contains a blob whose label does not match its bytes; expected {actual}",
                archive_path.display()
            )));
        }
        blobs.insert(label.to_vec(), content.to_vec());
    }
    for reference in <AppManifest as Realization>::references(&canonical).map_err(|error| {
        LiveError::InvalidHolo(format!(
            "read child application references {}: {error:?}",
            archive_path.display()
        ))
    })? {
        if !blobs.contains_key(reference.as_bytes()) {
            return Err(LiveError::Config(format!(
                "child archive {} is not self-contained; missing referenced object {reference}",
                archive_path.display()
            )));
        }
    }

    let capabilities_bytes = compile_capabilities(root, Some(&child.capabilities))?;
    let capabilities = address_bytes(&capabilities_bytes);
    blobs.insert(capabilities.as_bytes().to_vec(), capabilities_bytes);
    Ok(CompiledChild {
        application,
        capabilities,
        blobs,
    })
}

fn validate_child_source(child: &CompileChild) -> Result<()> {
    if child.application.as_os_str().is_empty() {
        return Err(LiveError::Config(
            "child application archive path cannot be empty".to_owned(),
        ));
    }
    if child.capabilities.as_os_str().is_empty() {
        return Err(LiveError::Config(
            "child delegated capability path cannot be empty".to_owned(),
        ));
    }
    Ok(())
}

fn build_layer(source: &CompileLayer, kappa: hologram::space::KappaLabel71) -> Result<Layer> {
    match source.kind {
        CompileLayerKind::Wasm => {
            reject_aux(source, "arch", source.arch.as_deref())?;
            reject_aux(source, "surface", source.surface.as_deref())?;
            reject_aux(source, "engine", source.engine.as_deref())?;
            let entry = source.entry.as_deref().ok_or_else(|| {
                layer_config_error(source, "Wasm layers require an explicit entry")
            })?;
            validate_entry_name(entry).map_err(|reason| {
                layer_config_error(source, &format!("invalid entry: {reason}"))
            })?;
            match source.contract.as_deref() {
                None => Err(layer_config_error(
                    source,
                    "Wasm layers require an explicit canonical contract",
                )),
                Some("") => Err(layer_config_error(
                    source,
                    "contract must be a non-empty canonical identifier",
                )),
                Some(contract) => {
                    let contract = normalize_wasm_contract(contract)
                        .map_err(|reason| layer_config_error(source, &reason))?;
                    if matches!(
                        contract,
                        WASM_CONTRACT_COMPONENT_V1
                            | WASM_CONTRACT_COMPONENT_STORE_READ_V1
                            | WASM_CONTRACT_COMPONENT_STORE_GRAPH_READ_V1
                            | WASM_CONTRACT_COMPONENT_STORE_WRITE_V1
                            | WASM_CONTRACT_COMPONENT_CHANNEL_PUBLISH_V1
                            | WASM_CONTRACT_COMPONENT_CHANNEL_SUBSCRIBE_V1
                            | WASM_CONTRACT_COMPONENT_NETWORK_FETCH_V1
                    ) && entry != COMPONENT_V1_ENTRY
                    {
                        return Err(layer_config_error(
                            source,
                            &format!(
                                "Component v1 entry must be {COMPONENT_V1_ENTRY:?}, got {entry:?}"
                            ),
                        ));
                    }
                    Ok(Layer::wasm_with_contract(kappa, entry, contract))
                }
            }
        }
        CompileLayerKind::Tensor => {
            reject_aux(source, "contract", source.contract.as_deref())?;
            reject_aux(source, "arch", source.arch.as_deref())?;
            reject_aux(source, "surface", source.surface.as_deref())?;
            reject_aux(source, "engine", source.engine.as_deref())?;
            Ok(Layer::tensor(
                kappa,
                effective_entry(source).unwrap_or("session"),
            ))
        }
        CompileLayerKind::Rootfs => {
            reject_aux(source, "contract", source.contract.as_deref())?;
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
            reject_aux(source, "contract", source.contract.as_deref())?;
            reject_aux(source, "arch", source.arch.as_deref())?;
            reject_aux(source, "engine", source.engine.as_deref())?;
            if source.entry.is_some() {
                return Err(layer_config_error(
                    source,
                    "view layers do not accept an entry field",
                ));
            }
            let surface = required_field(source, "surface", source.surface.as_deref())?;
            holo_view::validate_surface(surface).map_err(|_| {
                layer_config_error(
                    source,
                    &format!(
                        "unsupported surface {surface:?}; expected {:?}",
                        holo_view::PORTABLE_SURFACE
                    ),
                )
            })?;
            Ok(Layer::view(kappa, surface))
        }
        CompileLayerKind::InferenceModel => {
            reject_aux(source, "contract", source.contract.as_deref())?;
            reject_aux(source, "arch", source.arch.as_deref())?;
            reject_aux(source, "surface", source.surface.as_deref())?;
            let entry = required_field(source, "entry", source.entry.as_deref())?;
            let engine = required_field(source, "engine", source.engine.as_deref())?;
            Ok(Layer::inference_model(kappa, entry, engine))
        }
    }
}

fn compile_capabilities(root: &Path, path: Option<&Path>) -> Result<Vec<u8>> {
    path.map_or_else(
        || Ok(holo_capability::empty_canonical()),
        |path| {
            let resolved = root.join(path);
            let source =
                std::fs::read(&resolved).map_err(|error| LiveError::io(&resolved, error))?;
            holo_capability::compile_source(&resolved, &source)
        },
    )
}

fn read_required(root: &Path, path: &Path) -> Result<Vec<u8>> {
    let resolved = root.join(path);
    std::fs::read(&resolved).map_err(|error| LiveError::io(&resolved, error))
}

struct CompiledLayerContent {
    bytes: Vec<u8>,
    provenance: Option<PythonBuildProvenance>,
}

fn compile_layer_content(
    root: &Path,
    layer: &CompileLayer,
    options: CompileOptions,
) -> Result<CompiledLayerContent> {
    match (&layer.path, &layer.source) {
        (Some(path), None) => Ok(CompiledLayerContent {
            bytes: if matches!(layer.kind, CompileLayerKind::View) {
                holo_view::compile_directory(&root.join(path))?
            } else {
                read_required(root, path)?
            },
            provenance: None,
        }),
        (None, Some(CompileSource::Python(source))) => match source.profile {
            holo_python::PythonProfile::Rootfs => {
                let arch = required_field(layer, "arch", layer.arch.as_deref())?;
                let compiled = holo_python::compile(root, source, arch, options.no_build_cache)?;
                Ok(CompiledLayerContent {
                    bytes: compiled.bytes,
                    provenance: Some(PythonBuildProvenance::Rootfs(Box::new(compiled.provenance))),
                })
            }
            holo_python::PythonProfile::WasiComponent => {
                let compiled = holo_python_component::compile(root, source)?;
                Ok(CompiledLayerContent {
                    bytes: compiled.bytes,
                    provenance: Some(PythonBuildProvenance::Component(Box::new(
                        compiled.provenance,
                    ))),
                })
            }
        },
        _ => Err(layer_config_error(
            layer,
            "exactly one of path or source is required",
        )),
    }
}

fn validate_layer_source(layer: &CompileLayer) -> Result<()> {
    match (&layer.path, &layer.source) {
        (Some(path), None) if !path.as_os_str().is_empty() => Ok(()),
        (None, Some(CompileSource::Python(source))) => match source.profile {
            holo_python::PythonProfile::Rootfs => {
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
            holo_python::PythonProfile::WasiComponent => {
                if !matches!(layer.kind, CompileLayerKind::Wasm) {
                    return Err(layer_config_error(
                        layer,
                        "Python wasi-component sources require kind wasm",
                    ));
                }
                if layer.entry.as_deref() != Some(COMPONENT_V1_ENTRY)
                    || layer.contract.as_deref() != Some(WASM_CONTRACT_COMPONENT_V1)
                {
                    return Err(layer_config_error(
                        layer,
                        "Python wasi-component sources require entry \"run\" and contract \"hologram:guest/component@1\"",
                    ));
                }
                reject_aux(layer, "arch", layer.arch.as_deref())?;
                holo_python_component::validate_source(source)
            }
        },
        _ => Err(layer_config_error(
            layer,
            "exactly one of path or source is required",
        )),
    }
}

fn effective_entry(layer: &CompileLayer) -> Option<&str> {
    layer.entry.as_deref().or(match layer.source.as_ref() {
        Some(CompileSource::Python(source))
            if source.profile == holo_python::PythonProfile::Rootfs =>
        {
            Some(source.entry.as_str())
        }
        Some(CompileSource::Python(_)) | None => None,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application_plan::{explain_application, PlanLimits};
    use crate::holo::{inspect_bytes, plan_bytes};
    use crate::holo_capability;

    fn wasm_layer(content: KappaLabel71, entry: &str) -> Layer {
        Layer::wasm_with_contract(content, entry, crate::holo_contract::WASM_CONTRACT_CORE_V1)
    }

    fn write_child_archive(directory: &Path, include_blobs: bool) -> (PathBuf, KappaLabel71) {
        let capabilities = holo_capability::empty_canonical();
        let wasm = b"child wasm";
        let manifest = AppManifest {
            primary: Some(0),
            requires: address_bytes(&capabilities),
            layers: vec![wasm_layer(address_bytes(wasm), "holo_run")],
            children: Vec::new(),
        };
        let canonical = manifest.canonicalize();
        let application = address_bytes(&canonical);
        let mut writer = HoloWriter::new();
        writer.set_app_manifest(canonical);
        let capabilities_kappa = address_bytes(&capabilities);
        let wasm_kappa = address_bytes(wasm);
        let directory_blobs: Vec<(&[u8], &[u8])> = if include_blobs {
            vec![
                (capabilities_kappa.as_bytes(), capabilities.as_slice()),
                (wasm_kappa.as_bytes(), wasm.as_slice()),
            ]
        } else {
            Vec::new()
        };
        let application_directory =
            crate::holo_directory::derive(&manifest, directory_blobs.iter().copied())
                .expect("child directory");
        writer.add_extension(
            crate::holo_directory::DIRECTORY_EXTENSION_KEY,
            crate::holo_directory::encode(&application_directory).expect("encode child directory"),
        );
        if include_blobs {
            writer.add_content_blob(capabilities_kappa.as_bytes(), capabilities);
            writer.add_content_blob(wasm_kappa.as_bytes(), wasm);
        }
        let path = directory.join("worker.holo");
        std::fs::write(&path, writer.finish().expect("child archive")).expect("write child");
        (path, application)
    }

    #[test]
    fn compiles_a_self_contained_view_application() {
        let directory = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir(directory.path().join("ui")).expect("ui");
        std::fs::write(directory.path().join("ui/index.html"), "<h1>Hello</h1>").expect("view");
        std::fs::write(
            directory.path().join("hologram.json"),
            r#"{
                "schema_version": 4,
                "layers": [{"kind":"view","path":"ui","surface":"portable"}]
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

        let loader = HoloLoader::from_bytes(&compiled.bytes).expect("loader");
        let plan = loader.into_plan().expect("plan");
        let bundle = plan
            .content_blobs()
            .expect("content blobs")
            .into_iter()
            .map(|(_, bytes)| bytes)
            .find_map(|bytes| crate::holo_view::decode(bytes).ok())
            .expect("View bundle");
        assert_eq!(bundle.entry, crate::holo_view::PORTABLE_ENTRY);
        assert_eq!(bundle.files.len(), 1);
    }

    #[test]
    fn view_sources_require_a_directory_and_portable_surface() {
        let directory = tempfile::tempdir().expect("tempdir");
        std::fs::write(directory.path().join("index.html"), "<h1>Hello</h1>").expect("view");
        let manifest_path = directory.path().join("hologram.json");
        std::fs::write(
            &manifest_path,
            r#"{
                "schema_version": 4,
                "layers": [{"kind":"view","path":"index.html","surface":"portable"}]
            }"#,
        )
        .expect("manifest");
        let error = check_manifest(&manifest_path).expect_err("single file View must fail");
        assert!(error.to_string().contains("must be a directory"), "{error}");

        std::fs::create_dir(directory.path().join("ui")).expect("ui");
        std::fs::write(directory.path().join("ui/index.html"), "<h1>Hello</h1>").expect("entry");
        std::fs::write(
            &manifest_path,
            r#"{
                "schema_version": 4,
                "layers": [{"kind":"view","path":"ui","surface":"desktop"}]
            }"#,
        )
        .expect("manifest");
        let error = check_manifest(&manifest_path).expect_err("unsupported surface must fail");
        assert!(
            error.to_string().contains("expected \"portable\""),
            "{error}"
        );
    }

    #[test]
    fn fat_and_thin_packages_share_the_same_application_manifest() {
        let directory = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir(directory.path().join("ui")).expect("ui");
        std::fs::write(directory.path().join("ui/index.html"), "<h1>Hello</h1>").expect("view");
        let manifest_path = directory.path().join("hologram.json");
        std::fs::write(
            &manifest_path,
            r#"{
                "schema_version": 4,
                "layers": [{"kind":"view","path":"ui","surface":"portable"}]
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
    fn compiles_capability_source_to_canonical_content_for_fat_and_thin_archives() {
        let directory = tempfile::tempdir().expect("tempdir");
        std::fs::write(directory.path().join("app.wasm"), b"wasm bytes").expect("wasm");
        std::fs::write(
            directory.path().join("capabilities.json"),
            r#"{
                "schema_version": 2,
                "network_fetch_endpoints": ["https://api.example.com:443/v1"],
                "storage_quota_bytes": 4096
            }"#,
        )
        .expect("capabilities");
        let manifest_path = directory.path().join("hologram.json");
        std::fs::write(
            &manifest_path,
            r#"{
                "schema_version": 4,
                "primary": 0,
                "requires": "capabilities.json",
                "layers": [{"kind":"wasm","path":"app.wasm","entry":"holo_run","contract":"hologram:guest/core-wasm@1"}]
            }"#,
        )
        .expect("manifest");

        let checked = check_manifest(&manifest_path).expect("check");
        let fat = compile_manifest_with(&manifest_path, HoloPackaging::Fat).expect("fat");
        let thin = compile_manifest_with(&manifest_path, HoloPackaging::Thin).expect("thin");
        assert_eq!(checked.capabilities_kappa, fat.capabilities_kappa);
        assert_eq!(fat.capabilities_kappa, thin.capabilities_kappa);

        let loader = HoloLoader::from_bytes(&fat.bytes).expect("archive");
        let plan = loader.into_plan().expect("archive plan");
        let manifest = AppManifest::decode(plan.app_manifest().expect("application manifest"))
            .expect("decode application manifest");
        assert_eq!(manifest.requires.to_string(), fat.capabilities_kappa);
        let capabilities = plan
            .content_blobs()
            .expect("content blobs")
            .into_iter()
            .find_map(|(label, bytes)| (label == manifest.requires.as_bytes()).then_some(bytes))
            .expect("embedded capabilities");
        let decoded = holo_capability::decode_canonical(capabilities).expect("canonical set");
        assert_eq!(decoded.network_fetch_endpoints.len(), 1);
        assert_eq!(decoded.storage_quota_bytes, 4096);
    }

    #[test]
    fn check_reports_the_capability_source_path_and_field() {
        let directory = tempfile::tempdir().expect("tempdir");
        std::fs::write(directory.path().join("app.wasm"), b"wasm bytes").expect("wasm");
        std::fs::write(
            directory.path().join("capabilities.json"),
            r#"{"storage_roots":["not-a-kappa"]}"#,
        )
        .expect("capabilities");
        let manifest_path = directory.path().join("hologram.json");
        std::fs::write(
            &manifest_path,
            r#"{
                "schema_version": 4,
                "primary": 0,
                "requires": "capabilities.json",
                "layers": [{"kind":"wasm","path":"app.wasm","entry":"holo_run","contract":"hologram:guest/core-wasm@1"}]
            }"#,
        )
        .expect("manifest");

        let error = check_manifest(&manifest_path).expect_err("invalid capabilities");
        assert!(error.to_string().contains("capabilities.json"), "{error}");
        assert!(error.to_string().contains("storage_roots[0]"), "{error}");
    }

    #[test]
    fn current_schema_accepts_a_python_rootfs_source() {
        let manifest: CompileManifest = serde_json::from_str(
            r#"{
                "schema_version": 4,
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
    fn schema_four_accepts_an_import_free_python_component_source() {
        let manifest: CompileManifest = serde_json::from_str(
            r#"{
                "schema_version": 4,
                "primary": 0,
                "layers": [{
                    "kind": "wasm",
                    "source": {
                        "language": "python",
                        "project": ".",
                        "entry": "analytics:main",
                        "lock": "uv.lock",
                        "profile": "wasi-component"
                    },
                    "entry": "run",
                    "contract": "hologram:guest/component@1"
                }]
            }"#,
        )
        .expect("parse");
        validate_compile_manifest(&manifest).expect("validate");

        let mut wrong_contract = manifest;
        wrong_contract.layers[0].contract = None;
        let error = validate_compile_manifest(&wrong_contract).expect_err("contract required");
        assert!(error
            .to_string()
            .contains("require entry \"run\" and contract"));
    }

    #[test]
    fn current_schema_embeds_a_verified_child_archive_and_delegated_capabilities() {
        let directory = tempfile::tempdir().expect("tempdir");
        let (_, child_application) = write_child_archive(directory.path(), true);
        std::fs::write(directory.path().join("parent.wasm"), b"parent wasm").expect("parent");
        std::fs::write(
            directory.path().join("worker-capabilities.json"),
            r#"{"schema_version":2,"network_fetch_endpoints":["https://api.example.com:443/v1"]}"#,
        )
        .expect("delegated capabilities");
        let manifest_path = directory.path().join("hologram.json");
        std::fs::write(
            &manifest_path,
            r#"{
                "schema_version": 4,
                "primary": 0,
                "layers": [{"kind":"wasm","path":"parent.wasm","entry":"holo_run","contract":"hologram:guest/core-wasm@1"}],
                "children": [{
                    "application": "worker.holo",
                    "capabilities": "worker-capabilities.json"
                }]
            }"#,
        )
        .expect("manifest");

        let checked = check_manifest(&manifest_path).expect("check child source");
        assert_eq!(checked.specification.children.len(), 1);
        let fat = compile_manifest_with(&manifest_path, HoloPackaging::Fat).expect("fat");
        let thin = compile_manifest_with(&manifest_path, HoloPackaging::Thin).expect("thin");
        assert_eq!(fat.child_count, 1);
        assert_eq!(
            fat.identity.application_kappa,
            thin.identity.application_kappa
        );

        let inspection = inspect_bytes("parent", "parent.holo", &fat.bytes).expect("inspect");
        let application_directory = inspection.directory.expect("directory");
        assert_eq!(application_directory.children.len(), 1);
        assert_eq!(
            application_directory.children[0].application_kappa,
            child_application.to_string()
        );
        assert_eq!(application_directory.blobs.len(), 5);
        assert!(application_directory
            .blobs
            .iter()
            .any(|blob| blob.kappa == child_application.to_string()));
        let closure = explain_application(&fat.bytes, PlanLimits::default(), |_| Ok(None))
            .expect("resolve compiled child closure");
        assert_eq!(closure.application_count, 2);
        assert_eq!(closure.max_depth, 1);
        assert!(closure.blockers.is_empty());

        let thin_plan = HoloLoader::from_bytes(&thin.bytes)
            .expect("thin archive")
            .into_plan()
            .expect("thin plan");
        assert!(thin_plan.content_blobs().expect("thin blobs").is_empty());
    }

    #[test]
    fn child_sources_require_a_self_contained_archive() {
        let directory = tempfile::tempdir().expect("tempdir");
        write_child_archive(directory.path(), false);
        std::fs::write(directory.path().join("parent.wasm"), b"parent wasm").expect("parent");
        std::fs::write(
            directory.path().join("caps.json"),
            r#"{"schema_version":1}"#,
        )
        .expect("capabilities");
        let manifest_path = directory.path().join("hologram.json");
        std::fs::write(
            &manifest_path,
            r#"{
                "schema_version": 4,
                "primary": 0,
                "layers": [{"kind":"wasm","path":"parent.wasm","entry":"holo_run","contract":"hologram:guest/core-wasm@1"}],
                "children": [{"application":"worker.holo","capabilities":"caps.json"}]
            }"#,
        )
        .expect("manifest");
        let error = check_manifest(&manifest_path).expect_err("thin child must fail");
        assert!(error.to_string().contains("not self-contained"), "{error}");
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
                "schema_version": 4,
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
    fn wasm_layers_require_explicit_entry_and_contract() {
        let directory = tempfile::tempdir().expect("tempdir");
        std::fs::write(directory.path().join("app.wasm"), b"wasm bytes").expect("wasm");
        let manifest_path = directory.path().join("hologram.json");
        std::fs::write(
            &manifest_path,
            r#"{
                "schema_version": 4,
                "primary": 0,
                "layers": [{"kind":"wasm","path":"app.wasm"}]
            }"#,
        )
        .expect("manifest");
        let Err(error) = compile_manifest(&manifest_path) else {
            panic!("entry and contract are required")
        };
        assert!(error.to_string().contains("require an explicit entry"));

        let empty: CompileManifest = serde_json::from_str(
            r#"{
                "schema_version": 4,
                "primary": 0,
                "layers": [{"kind":"wasm","path":"app.wasm","entry":"","contract":"hologram:guest/core-wasm@1"}]
            }"#,
        )
        .expect("parse empty entry");
        let error = validate_compile_manifest(&empty).expect_err("empty entry");
        assert_eq!(error.code(), "LIVE_CONFIG_INVALID");
        assert!(error.to_string().contains("invalid entry"), "{error}");
    }

    #[test]
    fn schema_four_wasm_contract_is_identity_bearing_and_inspectable() {
        let directory = tempfile::tempdir().expect("tempdir");
        std::fs::write(directory.path().join("app.wasm"), b"wasm bytes").expect("wasm");
        let manifest_path = directory.path().join("hologram.json");
        std::fs::write(
            &manifest_path,
            r#"{
                "schema_version": 4,
                "primary": 0,
                "layers": [{"kind":"wasm","path":"app.wasm","entry":"holo_run","contract":"hologram:guest/core-wasm@1"}]
            }"#,
        )
        .expect("core manifest");
        let core = compile_manifest(&manifest_path).expect("compile core");

        std::fs::write(
            &manifest_path,
            r#"{
                "schema_version": 4,
                "primary": 0,
                "layers": [{
                    "kind":"wasm",
                    "path":"app.wasm",
                    "entry":"run",
                    "contract":"hologram:guest/component@1"
                }]
            }"#,
        )
        .expect("component manifest");
        let component = compile_manifest(&manifest_path).expect("compile component");
        assert_ne!(
            core.identity.application_kappa,
            component.identity.application_kappa
        );
        let inspection = inspect_bytes("component", "component.holo", &component.bytes)
            .expect("inspect component");
        assert_eq!(
            inspection.directory.expect("directory").layers[0]
                .contract
                .as_deref(),
            Some(crate::holo_contract::WASM_CONTRACT_COMPONENT_V1)
        );
        let plan = plan_bytes(&component.bytes).expect("plan component");
        assert_eq!(
            plan.layers[0].contract.as_deref(),
            Some(crate::holo_contract::WASM_CONTRACT_COMPONENT_V1)
        );
        assert!(plan.runnable);
        assert_eq!(
            plan.layers[0].provider.name.as_deref(),
            Some("wasmtime-component-direct")
        );
        assert!(plan.blockers.is_empty());

        std::fs::write(
            &manifest_path,
            r#"{
                "schema_version": 4,
                "primary": 0,
                "layers": [{
                    "kind":"wasm",
                    "path":"app.wasm",
                    "entry":"run",
                    "contract":"hologram:guest/component-store-read@1"
                }]
            }"#,
        )
        .expect("store-read manifest");
        let store_read = compile_manifest(&manifest_path).expect("compile store-read profile");
        let plan = plan_bytes(&store_read.bytes).expect("plan store-read profile");
        assert_eq!(
            plan.layers[0].contract.as_deref(),
            Some(crate::holo_contract::WASM_CONTRACT_COMPONENT_STORE_READ_V1)
        );
        assert_eq!(
            plan.layers[0].provider.name.as_deref(),
            Some("wasmtime-component-store-read-direct")
        );
        assert!(plan.runnable);

        std::fs::write(
            &manifest_path,
            r#"{
                "schema_version": 4,
                "primary": 0,
                "layers": [{
                    "kind":"wasm",
                    "path":"app.wasm",
                    "entry":"run",
                    "contract":"hologram:guest/component-store-graph-read@1"
                }]
            }"#,
        )
        .expect("store-graph-read manifest");
        let store_graph_read =
            compile_manifest(&manifest_path).expect("compile store-graph-read profile");
        let plan = plan_bytes(&store_graph_read.bytes).expect("plan store-graph-read profile");
        assert_eq!(
            plan.layers[0].contract.as_deref(),
            Some(crate::holo_contract::WASM_CONTRACT_COMPONENT_STORE_GRAPH_READ_V1)
        );
        assert_eq!(
            plan.layers[0].provider.name.as_deref(),
            Some("wasmtime-component-store-graph-read-direct")
        );
        assert!(plan.runnable);

        std::fs::write(
            &manifest_path,
            r#"{
                "schema_version": 4,
                "primary": 0,
                "layers": [{
                    "kind":"wasm",
                    "path":"app.wasm",
                    "entry":"run",
                    "contract":"hologram:guest/component-store-write@1"
                }]
            }"#,
        )
        .expect("store-write manifest");
        let store_write = compile_manifest(&manifest_path).expect("compile store-write profile");
        let plan = plan_bytes(&store_write.bytes).expect("plan store-write profile");
        assert_eq!(
            plan.layers[0].contract.as_deref(),
            Some(crate::holo_contract::WASM_CONTRACT_COMPONENT_STORE_WRITE_V1)
        );
        assert_eq!(
            plan.layers[0].provider.name.as_deref(),
            Some("wasmtime-component-store-write-direct")
        );
        assert!(plan.runnable);

        for (contract, expected_provider) in [
            (
                crate::holo_contract::WASM_CONTRACT_COMPONENT_CHANNEL_PUBLISH_V1,
                "wasmtime-component-channel-publish-direct",
            ),
            (
                crate::holo_contract::WASM_CONTRACT_COMPONENT_CHANNEL_SUBSCRIBE_V1,
                "wasmtime-component-channel-subscribe-direct",
            ),
            (
                crate::holo_contract::WASM_CONTRACT_COMPONENT_NETWORK_FETCH_V1,
                "wasmtime-component-network-fetch-direct",
            ),
        ] {
            std::fs::write(
                &manifest_path,
                format!(
                    r#"{{
                        "schema_version": 4,
                        "primary": 0,
                        "layers": [{{
                            "kind":"wasm",
                            "path":"app.wasm",
                            "entry":"run",
                            "contract":"{contract}"
                        }}]
                    }}"#
                ),
            )
            .expect("channel manifest");
            let compiled = compile_manifest(&manifest_path).expect("compile channel profile");
            let plan = plan_bytes(&compiled.bytes).expect("plan channel profile");
            assert_eq!(plan.layers[0].contract.as_deref(), Some(contract));
            assert_eq!(
                plan.layers[0].provider.name.as_deref(),
                Some(expected_provider)
            );
            assert!(plan.runnable);
        }

        let omitted: CompileManifest = serde_json::from_str(
            r#"{
                "schema_version": 4,
                "primary": 0,
                "layers": [{"kind":"wasm","path":"app.wasm","entry":"holo_run"}]
            }"#,
        )
        .expect("parse omitted contract");
        let error = validate_compile_manifest(&omitted).expect_err("contract is required");
        assert!(error.to_string().contains("explicit canonical contract"));
    }

    #[test]
    fn noncurrent_source_schema_and_unknown_contract_fail_closed() {
        let noncurrent: CompileManifest = serde_json::from_str(
            r#"{
                "schema_version": 3,
                "primary": 0,
                "layers": [{
                    "kind":"wasm",
                    "path":"app.wasm",
                    "contract":"hologram:guest/core-wasm@1"
                }]
            }"#,
        )
        .expect("parse noncurrent schema");
        let error = validate_compile_manifest(&noncurrent).expect_err("schema mismatch");
        assert!(error.to_string().contains("expected 4"), "{error}");

        let unknown: CompileManifest = serde_json::from_str(
            r#"{
                "schema_version": 4,
                "primary": 0,
                "layers": [{
                    "kind":"wasm",
                    "path":"app.wasm",
                    "entry":"holo_run",
                    "contract":"hologram:guest/core-wasm@2"
                }]
            }"#,
        )
        .expect("parse unknown contract");
        let error = validate_compile_manifest(&unknown).expect_err("unknown contract");
        assert!(error
            .to_string()
            .contains("unsupported Wasm guest contract"));

        let wrong_entry: CompileManifest = serde_json::from_str(
            r#"{
                "schema_version": 4,
                "primary": 0,
                "layers": [{
                    "kind":"wasm",
                    "path":"app.wasm",
                    "entry":"holo_run",
                    "contract":"hologram:guest/component@1"
                }]
            }"#,
        )
        .expect("parse wrong component entry");
        let error = validate_compile_manifest(&wrong_entry).expect_err("wrong component entry");
        assert!(error
            .to_string()
            .contains("Component v1 entry must be \"run\""));
    }

    #[test]
    fn inference_model_layers_require_an_entry_and_engine() {
        let missing_engine: CompileManifest = serde_json::from_str(
            r#"{
                "schema_version": 4,
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
                "schema_version": 4,
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
