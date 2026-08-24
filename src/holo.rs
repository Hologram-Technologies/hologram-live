use crate::error::{LiveError, Result};
use crate::protocol::{HoloInspection, HoloRunResult, HoloSection, ResidentHolo};
use crate::store::ObjectStore;
use crate::util::hex;
use hologram::archive::{HoloLoader, HoloWriter};
use std::sync::Arc;

const HOLO_MEDIA_TYPE: &str = "application/vnd.hologram.holo";

/// Stable, content-addressed catalog support for `.holo` archives.
///
/// The default Hologram Live build intentionally depends only on the archive
/// surface of the pinned Hologram revision. The pinned upstream x86 CPU runtime
/// currently compiles AVX-512 intrinsics that require unstable Rust APIs; Live
/// refuses to hide that behind `RUSTC_BOOTSTRAP`. Runtime execution remains an
/// explicit capability seam until the upstream stable-toolchain issue is fixed.
pub struct HoloCatalog {
    store: Arc<ObjectStore>,
}

impl HoloCatalog {
    pub fn new(store: Arc<ObjectStore>) -> Self {
        Self { store }
    }

    pub fn import(&self, name: String, bytes: Vec<u8>) -> Result<HoloInspection> {
        let inspection = inspect_bytes("pending", &name, &bytes)?;
        let metadata = self
            .store
            .put("holo", HOLO_MEDIA_TYPE, Some(name.clone()), &bytes)?;
        Ok(HoloInspection {
            kappa: metadata.id,
            name,
            ..inspection
        })
    }

    pub fn list(&self) -> Result<Vec<HoloInspection>> {
        self.store
            .list(Some("holo"))?
            .into_iter()
            .map(|metadata| {
                let bytes = self.store.get(&metadata.id)?;
                inspect_bytes(
                    &metadata.id,
                    metadata.filename.as_deref().unwrap_or("application.holo"),
                    &bytes,
                )
            })
            .collect()
    }

    pub fn inspect(&self, kappa: &str) -> Result<HoloInspection> {
        let metadata = self.store.metadata(kappa)?;
        if metadata.kind != "holo" {
            return Err(LiveError::InvalidHolo(format!(
                "object {kappa} is not cataloged as a .holo archive"
            )));
        }
        let bytes = self.store.get(kappa)?;
        inspect_bytes(
            kappa,
            metadata.filename.as_deref().unwrap_or("application.holo"),
            &bytes,
        )
    }

    pub fn verify(&self, kappa: &str) -> Result<HoloInspection> {
        if !self.store.verify(kappa)? {
            return Err(LiveError::InvalidHolo(format!(
                "content address verification failed for {kappa}"
            )));
        }
        self.inspect(kappa)
    }

    pub fn bytes(&self, kappa: &str) -> Result<Vec<u8>> {
        self.verify(kappa)?;
        self.store.get(kappa)
    }

    pub fn remove(&self, kappa: &str) -> Result<()> {
        self.store.remove_metadata(kappa)
    }

    /// Produce the smallest structurally valid archive for smoke tests and
    /// parser demonstrations. It is intentionally non-executable.
    pub fn fixture() -> Result<Vec<u8>> {
        HoloWriter::new()
            .finish()
            .map_err(|error| LiveError::InvalidHolo(error.to_string()))
    }
}

pub fn inspect_bytes(kappa: &str, name: &str, bytes: &[u8]) -> Result<HoloInspection> {
    let loader =
        HoloLoader::from_bytes(bytes).map_err(|error| LiveError::InvalidHolo(error.to_string()))?;
    let fingerprint = loader.fingerprint();
    let plan = loader
        .into_plan()
        .map_err(|error| LiveError::InvalidHolo(error.to_string()))?;
    let format_version = u16::from_le_bytes([bytes[4], bytes[5]]);
    let sections = plan
        .sections()
        .iter()
        .map(|section| HoloSection {
            kind: format!("{:?}", section.kind),
            offset: section.offset,
            length: section.length,
        })
        .collect();
    Ok(HoloInspection {
        kappa: kappa.to_owned(),
        name: name.to_owned(),
        format_version,
        byte_length: bytes.len().try_into().unwrap_or(u64::MAX),
        archive_fingerprint: hex(&fingerprint),
        footer_verified: true,
        sections,
    })
}

/// Execution-provider seam retained by the module API.
///
/// The stable v1 default advertises archive operations only, so these methods
/// are never selected by capability-aware clients. Direct callers still get a
/// typed, honest `LIVE_CAPABILITY_MISSING` error instead of a silent fallback.
pub struct HoloRuntime {
    _catalog: Arc<HoloCatalog>,
}

impl HoloRuntime {
    pub fn new(catalog: Arc<HoloCatalog>, _mailbox_capacity: usize) -> Self {
        Self { _catalog: catalog }
    }

    pub async fn load(&self, kappa: &str) -> Result<ResidentHolo> {
        Err(runtime_unavailable(kappa))
    }

    pub async fn unload(&self, kappa: &str) -> Result<()> {
        Err(runtime_unavailable(kappa))
    }

    pub async fn run(&self, kappa: &str, _inputs: Vec<Vec<u8>>) -> Result<HoloRunResult> {
        Err(runtime_unavailable(kappa))
    }

    pub async fn list(&self) -> Result<Vec<ResidentHolo>> {
        Ok(Vec::new())
    }
}

fn runtime_unavailable(kappa: &str) -> LiveError {
    LiveError::Capability(format!(
        ".holo execution for {kappa} is not compiled into the stable v1 build; \
         import, inspect, and verify are available"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_is_a_valid_holo_archive() {
        let bytes = HoloCatalog::fixture().expect("fixture");
        let inspection = inspect_bytes("fixture", "fixture.holo", &bytes).expect("inspect");
        assert!(inspection.footer_verified);
        assert_eq!(inspection.format_version, 3);
    }

    #[tokio::test]
    async fn execution_fails_with_typed_capability_error() {
        let temporary =
            std::env::temp_dir().join(format!("hologram-live-holo-test-{}", std::process::id()));
        let store = Arc::new(ObjectStore::open(temporary).expect("store"));
        let runtime = HoloRuntime::new(Arc::new(HoloCatalog::new(store)), 1);
        let error = runtime
            .run("blake3:test", Vec::new())
            .await
            .expect_err("must fail");
        assert_eq!(error.code(), "LIVE_CAPABILITY_MISSING");
    }
}
