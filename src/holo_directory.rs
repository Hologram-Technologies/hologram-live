//! Queryable application-directory projection for `.holo` v3 archives.

use crate::error::{LiveError, Result};
use crate::protocol::{HoloBlob, HoloChild, HoloDirectory, HoloLayer};
use hologram::space::{address_bytes, AppManifest, LayerKind};
use std::collections::BTreeMap;

/// Extension key carrying the normalized application directory. The archive's
/// canonical `AppManifest` remains the source of identity and execution truth.
pub const DIRECTORY_EXTENSION_KEY: &str =
    "https://hologram.foundation/extension/application-directory/v1";
pub const DIRECTORY_SCHEMA_VERSION: u16 = 1;

/// Derive the directory from the canonical manifest and physical blob table.
/// Every embedded blob label is re-derived before it becomes queryable.
pub fn derive<'a, I>(manifest: &AppManifest, blobs: I) -> Result<HoloDirectory>
where
    I: IntoIterator<Item = (&'a [u8], &'a [u8])>,
{
    manifest.validate().map_err(|error| {
        LiveError::InvalidHolo(format!("invalid application manifest: {error:?}"))
    })?;

    let mut indexed_blobs = BTreeMap::new();
    for (label, content) in blobs {
        let label = std::str::from_utf8(label)
            .map_err(|_| LiveError::InvalidHolo("content blob kappa is not UTF-8".to_owned()))?;
        let expected = address_bytes(content);
        if expected.as_str() != label {
            return Err(LiveError::InvalidHolo(format!(
                "content blob {label} does not match its bytes; expected {expected}"
            )));
        }
        let length = u64::try_from(content.len()).unwrap_or(u64::MAX);
        if indexed_blobs.insert(label.to_owned(), length).is_some() {
            return Err(LiveError::InvalidHolo(format!(
                "content blob {label} is embedded more than once"
            )));
        }
    }

    let layers = manifest
        .layers
        .iter()
        .enumerate()
        .map(|(position, layer)| {
            let position = u32::try_from(position).map_err(|_| {
                LiveError::InvalidHolo("application has too many layers".to_owned())
            })?;
            let (kind, architecture, surface) = match layer.kind {
                LayerKind::WasmCodemodule => ("wasm", None, None),
                LayerKind::TensorPlan => ("tensor", None, None),
                LayerKind::RootfsImage => ("rootfs", Some(layer.aux.clone()), None),
                LayerKind::View => ("view", None, Some(layer.aux.clone())),
            };
            Ok(HoloLayer {
                position,
                kind: kind.to_owned(),
                content_kappa: layer.content.to_string(),
                entry: layer.entry.clone(),
                architecture,
                surface,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let children = manifest
        .children
        .iter()
        .enumerate()
        .map(|(position, (application, capabilities))| {
            Ok(HoloChild {
                position: u32::try_from(position).map_err(|_| {
                    LiveError::InvalidHolo("application has too many children".to_owned())
                })?,
                application_kappa: application.to_string(),
                capabilities_kappa: capabilities.to_string(),
            })
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(HoloDirectory {
        schema_version: DIRECTORY_SCHEMA_VERSION,
        primary_layer: manifest.primary,
        requires_kappa: manifest.requires.to_string(),
        layers,
        children,
        blobs: indexed_blobs
            .into_iter()
            .map(|(kappa, byte_length)| HoloBlob { kappa, byte_length })
            .collect(),
    })
}

pub fn encode(directory: &HoloDirectory) -> Result<Vec<u8>> {
    serde_json::to_vec(directory).map_err(Into::into)
}

pub fn decode(bytes: &[u8]) -> Result<HoloDirectory> {
    let directory: HoloDirectory = serde_json::from_slice(bytes).map_err(|error| {
        LiveError::InvalidHolo(format!("decode application directory: {error}"))
    })?;
    if directory.schema_version != DIRECTORY_SCHEMA_VERSION {
        return Err(LiveError::InvalidHolo(format!(
            "unsupported application directory schema {}; expected {DIRECTORY_SCHEMA_VERSION}",
            directory.schema_version
        )));
    }
    Ok(directory)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hologram::space::Layer;

    #[test]
    fn directory_is_normalized_and_blob_order_is_deterministic() {
        let first = b"first";
        let second = b"second";
        let manifest = AppManifest {
            primary: Some(0),
            requires: address_bytes(first),
            layers: vec![Layer::wasm(address_bytes(second), "holo_run")],
            children: Vec::new(),
        };

        let directory = derive(
            &manifest,
            [
                (address_bytes(second), second.as_slice()),
                (address_bytes(first), first.as_slice()),
            ]
            .iter()
            .map(|(kappa, content)| (kappa.as_bytes(), *content)),
        )
        .expect("directory");

        assert!(directory.blobs[0].kappa < directory.blobs[1].kappa);
        assert_eq!(directory.layers[0].position, 0);
        assert_eq!(
            directory.layers[0].content_kappa,
            address_bytes(second).to_string()
        );
    }

    #[test]
    fn duplicate_blob_labels_are_rejected() {
        let content = b"same";
        let kappa = address_bytes(content);
        let manifest = AppManifest {
            primary: Some(0),
            requires: kappa,
            layers: vec![Layer::wasm(kappa, "holo_run")],
            children: Vec::new(),
        };
        let error = derive(&manifest, [(kappa.as_bytes(), content.as_slice()); 2])
            .expect_err("duplicate must fail");

        assert!(error.to_string().contains("more than once"));
    }

    #[test]
    fn unknown_directory_schema_is_rejected() {
        let error = decode(
            br#"{"schema_version":2,"primary_layer":null,"requires_kappa":"blake3:none","layers":[],"children":[],"blobs":[]}"#,
        )
        .expect_err("future schema must fail closed");

        assert!(error
            .to_string()
            .contains("unsupported application directory schema"));
    }
}
