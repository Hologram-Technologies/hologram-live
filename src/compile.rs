use crate::error::{LiveError, Result};
use hologram::archive::HoloWriter;
use hologram::space::{address_bytes, AppManifest, Layer, Realization};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const MANIFEST_SCHEMA_VERSION: u16 = 1;

/// Source manifest accepted by `hologram compile`.
///
/// Paths are resolved relative to the manifest file. The resulting archive is
/// a self-contained Hologram v3 application: every layer and the declared
/// capability set are embedded under their canonical kappa labels.
#[derive(Debug, Deserialize)]
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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompileLayer {
    pub kind: CompileLayerKind,
    pub path: PathBuf,
    #[serde(default)]
    pub entry: Option<String>,
    #[serde(default)]
    pub arch: Option<String>,
    #[serde(default)]
    pub surface: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CompileLayerKind {
    Wasm,
    Tensor,
    Rootfs,
    View,
}

pub struct CompiledHolo {
    pub bytes: Vec<u8>,
    pub layer_count: usize,
}

pub fn compile_manifest(path: &Path) -> Result<CompiledHolo> {
    let source = std::fs::read(path).map_err(|error| LiveError::io(path, error))?;
    let specification: CompileManifest = serde_json::from_slice(&source).map_err(|error| {
        LiveError::Config(format!(
            "parse compile manifest {}: {error}",
            path.display()
        ))
    })?;
    if specification.schema_version != MANIFEST_SCHEMA_VERSION {
        return Err(LiveError::Config(format!(
            "unsupported compile manifest schema {}; expected {MANIFEST_SCHEMA_VERSION}",
            specification.schema_version
        )));
    }

    let root = path.parent().unwrap_or_else(|| Path::new("."));
    let requires_bytes = read_relative(root, specification.requires.as_deref())?;
    let requires = address_bytes(&requires_bytes);
    let mut blobs = BTreeMap::new();
    blobs.insert(requires.as_bytes().to_vec(), requires_bytes);

    let mut layers = Vec::with_capacity(specification.layers.len());
    for source_layer in &specification.layers {
        let content = read_required(root, &source_layer.path)?;
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
    let mut writer = HoloWriter::new();
    writer.set_app_manifest(manifest.canonicalize());
    writer.set_metadata(source);
    for (kappa, content) in blobs {
        writer.add_content_blob(kappa, content);
    }
    let bytes = writer
        .finish()
        .map_err(|error| LiveError::InvalidHolo(error.to_string()))?;
    Ok(CompiledHolo { bytes, layer_count })
}

fn build_layer(source: &CompileLayer, kappa: hologram::space::KappaLabel71) -> Result<Layer> {
    match source.kind {
        CompileLayerKind::Wasm => {
            reject_aux(source, "arch", source.arch.as_deref())?;
            reject_aux(source, "surface", source.surface.as_deref())?;
            Ok(Layer::wasm(
                kappa,
                source.entry.as_deref().unwrap_or("_start"),
            ))
        }
        CompileLayerKind::Tensor => {
            reject_aux(source, "arch", source.arch.as_deref())?;
            reject_aux(source, "surface", source.surface.as_deref())?;
            Ok(Layer::tensor(
                kappa,
                source.entry.as_deref().unwrap_or("session"),
            ))
        }
        CompileLayerKind::Rootfs => {
            reject_aux(source, "surface", source.surface.as_deref())?;
            let arch = required_field(source, "arch", source.arch.as_deref())?;
            Ok(Layer::rootfs(
                kappa,
                source.entry.as_deref().unwrap_or("boot"),
                arch,
            ))
        }
        CompileLayerKind::View => {
            reject_aux(source, "arch", source.arch.as_deref())?;
            if source.entry.is_some() {
                return Err(layer_config_error(
                    source,
                    "view layers do not accept an entry field",
                ));
            }
            let surface = required_field(source, "surface", source.surface.as_deref())?;
            Ok(Layer::view(kappa, surface))
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
        layer.path.display(),
        kind(layer)
    ))
}

const fn kind(layer: &CompileLayer) -> &'static str {
    match layer.kind {
        CompileLayerKind::Wasm => "wasm",
        CompileLayerKind::Tensor => "tensor",
        CompileLayerKind::Rootfs => "rootfs",
        CompileLayerKind::View => "view",
    }
}

const fn default_schema_version() -> u16 {
    MANIFEST_SCHEMA_VERSION
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
        let inspection = inspect_bytes("local", "hello.holo", &compiled.bytes).expect("inspect");
        assert!(inspection
            .sections
            .iter()
            .any(|section| section.kind == "AppManifest"));
        assert!(inspection
            .sections
            .iter()
            .any(|section| section.kind == "ContentBlob"));
    }
}
