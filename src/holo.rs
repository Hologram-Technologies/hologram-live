use crate::actor::ActorSystem;
use crate::error::{LiveError, Result};
use crate::holo_directory::{self, DIRECTORY_EXTENSION_KEY};
use crate::holo_wasm::{ResidentHoloActor, Run};
use crate::protocol::{HoloInspection, HoloRunResult, HoloSection, ResidentHolo};
use crate::store::ObjectStore;
use crate::util::hex;
use hologram::archive::{HoloLoader, HoloWriter};
use hologram::space::{AppManifest, LayerKind};
use kameo::actor::{ActorRef, Spawn};
use kameo::error::SendError;
use kameo::mailbox;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use wasmtime::Engine;

const HOLO_MEDIA_TYPE: &str = "application/vnd.hologram.holo";

/// Stable, content-addressed catalog support for `.holo` archives.
///
/// The default Hologram Live build intentionally depends only on the archive
/// surface of the pinned Hologram revision. The pinned upstream x86 CPU runtime
/// currently compiles AVX-512 intrinsics that require unstable Rust APIs; Live
/// refuses to hide that behind `RUSTC_BOOTSTRAP`. Execution of primary wasm
/// layers runs in-process through wasmtime instead (see [`HoloRuntime`] and
/// `crate::holo_wasm`); tensor and rootfs layers remain explicit capability
/// seams until the upstream stable-toolchain issue is fixed.
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
    let extensions = plan
        .extensions()
        .map_err(|error| LiveError::InvalidHolo(error.to_string()))?;
    let directory_extensions = extensions
        .iter()
        .filter(|(key, _)| *key == DIRECTORY_EXTENSION_KEY)
        .collect::<Vec<_>>();
    if directory_extensions.len() > 1 {
        return Err(LiveError::InvalidHolo(
            "archive contains more than one application directory".to_owned(),
        ));
    }
    let directory_embedded = !directory_extensions.is_empty();
    let directory = if let Some(manifest_bytes) = plan.app_manifest() {
        let manifest = AppManifest::decode(manifest_bytes).map_err(|error| {
            LiveError::InvalidHolo(format!("decode application manifest: {error:?}"))
        })?;
        let blobs = plan
            .content_blobs()
            .map_err(|error| LiveError::InvalidHolo(error.to_string()))?;
        let derived = holo_directory::derive(&manifest, blobs.iter().copied())?;
        if let Some((_, bytes)) = directory_extensions.first() {
            let declared = holo_directory::decode(bytes)?;
            if declared != derived {
                return Err(LiveError::InvalidHolo(
                    "application directory does not match the manifest and embedded blobs"
                        .to_owned(),
                ));
            }
        }
        Some(derived)
    } else {
        if directory_embedded {
            return Err(LiveError::InvalidHolo(
                "application directory requires an application manifest".to_owned(),
            ));
        }
        None
    };
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
        directory,
        directory_embedded,
    })
}

/// In-process execution provider for `.holo` archives.
///
/// v1 executes archives containing exactly one primary wasm layer
/// (see `crate::holo_wasm` for the guest contract); tensor, rootfs, and view
/// layers remain typed `LIVE_CAPABILITY_MISSING` seams. Loading a kappa
/// spawns a resident `ResidentHoloActor` under the runtime's own supervision
/// root; `run` messages it, `unload` stops it.
///
/// The runtime spawns its own `ActorSystem` lazily on first load: the
/// daemon's root supervisor is created after `HoloRuntime` in
/// `AppState::build`, so the handle cannot be threaded through the
/// constructor without changing `app.rs`.
pub struct HoloRuntime {
    catalog: Arc<HoloCatalog>,
    mailbox_capacity: usize,
    engine: Engine,
    actors: OnceLock<ActorSystem>,
    resident: Mutex<HashMap<String, ResidentEntry>>,
}

/// Cloning is cheap: the actor reference and counters are shared handles.
#[derive(Clone)]
struct ResidentEntry {
    actor: ActorRef<ResidentHoloActor>,
    input_count: usize,
    output_count: usize,
    resident_bytes: usize,
    queued: Arc<AtomicUsize>,
    processed: Arc<AtomicUsize>,
}

impl ResidentEntry {
    fn record(&self, kappa: &str) -> ResidentHolo {
        ResidentHolo {
            kappa: kappa.to_owned(),
            input_count: self.input_count,
            output_count: self.output_count,
            resident_bytes: self.resident_bytes,
            queued: self.queued.load(Ordering::Relaxed),
            processed: self.processed.load(Ordering::Relaxed),
        }
    }
}

impl HoloRuntime {
    pub fn new(catalog: Arc<HoloCatalog>, mailbox_capacity: usize) -> Self {
        Self {
            catalog,
            mailbox_capacity: mailbox_capacity.max(1),
            engine: Engine::default(),
            actors: OnceLock::new(),
            resident: Mutex::new(HashMap::new()),
        }
    }

    pub async fn load(&self, kappa: &str) -> Result<ResidentHolo> {
        if let Some(record) = self.resident_record(kappa)? {
            return Ok(record);
        }
        let bytes = self.catalog.bytes(kappa)?;
        let wasm = extract_primary_wasm(kappa, &bytes)?;
        let queued = Arc::new(AtomicUsize::new(0));
        let processed = Arc::new(AtomicUsize::new(0));
        let actor = ResidentHoloActor::compile(kappa, &self.engine, &wasm, processed.clone())?;
        let actor = ResidentHoloActor::spawn_link_with_mailbox(
            self.actors.get_or_init(ActorSystem::start).root(),
            actor,
            mailbox::bounded(self.mailbox_capacity),
        )
        .await;
        // The v1 manifest carries no I/O arity, so the contract's
        // one-output-per-input shape is reported as 1/1.
        let entry = ResidentEntry {
            actor,
            input_count: 1,
            output_count: 1,
            resident_bytes: wasm.len(),
            queued,
            processed,
        };
        let mut resident = self.lock_resident()?;
        Ok(match resident.entry(kappa.to_owned()) {
            std::collections::hash_map::Entry::Occupied(existing) => existing.get().record(kappa),
            std::collections::hash_map::Entry::Vacant(slot) => slot.insert(entry).record(kappa),
        })
    }

    pub async fn unload(&self, kappa: &str) -> Result<()> {
        let entry = self
            .lock_resident()?
            .remove(kappa)
            .ok_or_else(|| not_resident(kappa))?;
        let _ = entry.actor.stop_gracefully().await;
        entry.actor.wait_for_shutdown().await;
        Ok(())
    }

    pub async fn run(&self, kappa: &str, inputs: Vec<Vec<u8>>) -> Result<HoloRunResult> {
        let entry = self.entry(kappa)?.ok_or_else(|| not_resident(kappa))?;
        entry.queued.fetch_add(1, Ordering::Relaxed);
        let reply = entry.actor.ask(Run { inputs }).await;
        entry.queued.fetch_sub(1, Ordering::Relaxed);
        let outcome = match reply {
            Ok(outcome) => outcome,
            Err(SendError::HandlerError(error)) => return Err(error),
            Err(error) => {
                return Err(LiveError::Conflict(format!(
                    "resident holo {kappa} is unavailable: {error}"
                )));
            }
        };
        Ok(HoloRunResult {
            kappa: kappa.to_owned(),
            outputs: outcome.outputs,
            elapsed_micros: outcome.elapsed_micros,
            resident_bytes: entry.resident_bytes,
        })
    }

    pub async fn list(&self) -> Result<Vec<ResidentHolo>> {
        Ok(self
            .lock_resident()?
            .iter()
            .map(|(kappa, entry)| entry.record(kappa))
            .collect())
    }

    fn resident_record(&self, kappa: &str) -> Result<Option<ResidentHolo>> {
        Ok(self
            .lock_resident()?
            .get(kappa)
            .map(|entry| entry.record(kappa)))
    }

    fn entry(&self, kappa: &str) -> Result<Option<ResidentEntry>> {
        Ok(self.lock_resident()?.get(kappa).cloned())
    }

    fn lock_resident(&self) -> Result<std::sync::MutexGuard<'_, HashMap<String, ResidentEntry>>> {
        self.resident
            .lock()
            .map_err(|_| LiveError::Conflict("resident holo registry lock poisoned".to_owned()))
    }
}

fn not_resident(kappa: &str) -> LiveError {
    LiveError::NotFound(format!(
        "{kappa} is not loaded as a resident holo; run `hologram holo load {kappa}` first"
    ))
}

/// Extract the wasm payload of an archive that is exactly one primary wasm
/// layer. Anything else is an honest capability error naming the unsupported
/// layer kinds.
fn extract_primary_wasm(kappa: &str, bytes: &[u8]) -> Result<Vec<u8>> {
    let loader =
        HoloLoader::from_bytes(bytes).map_err(|error| LiveError::InvalidHolo(error.to_string()))?;
    let plan = loader
        .into_plan()
        .map_err(|error| LiveError::InvalidHolo(error.to_string()))?;
    let manifest_bytes = plan.app_manifest().ok_or_else(|| {
        LiveError::Capability(format!(
            "holo.load for {kappa} requires an application manifest with a primary wasm layer"
        ))
    })?;
    let manifest = AppManifest::decode(manifest_bytes).map_err(|error| {
        LiveError::InvalidHolo(format!("decode application manifest of {kappa}: {error:?}"))
    })?;
    let unsupported: Vec<&'static str> = manifest
        .layers
        .iter()
        .filter(|layer| layer.kind != LayerKind::WasmCodemodule)
        .map(|layer| layer_kind_name(layer.kind))
        .collect();
    if !unsupported.is_empty() {
        return Err(LiveError::Capability(format!(
            "holo.load for {kappa} supports wasm layers only; the archive contains unsupported \
             layer kinds: {}",
            unsupported.join(", ")
        )));
    }
    if manifest.layers.len() != 1 {
        return Err(LiveError::Capability(format!(
            "holo.load for {kappa} requires exactly one wasm layer; the archive declares {} layers",
            manifest.layers.len()
        )));
    }
    if manifest.primary != Some(0) {
        return Err(LiveError::Capability(format!(
            "holo.load for {kappa} requires the wasm layer to be the archive's primary layer"
        )));
    }
    let content = manifest.layers[0].content;
    let blobs = plan
        .content_blobs()
        .map_err(|error| LiveError::InvalidHolo(error.to_string()))?;
    let (_, blob) = blobs
        .iter()
        .find(|(label, _)| *label == content.as_bytes())
        .ok_or_else(|| {
            LiveError::InvalidHolo(format!(
                "wasm layer content of {kappa} is not embedded in the archive"
            ))
        })?;
    Ok(blob.to_vec())
}

const fn layer_kind_name(kind: LayerKind) -> &'static str {
    match kind {
        LayerKind::WasmCodemodule => "wasm",
        LayerKind::TensorPlan => "tensor",
        LayerKind::RootfsImage => "rootfs",
        LayerKind::View => "view",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hologram::space::{address_bytes, Layer, Realization};

    #[test]
    fn fixture_is_a_valid_holo_archive() {
        let bytes = HoloCatalog::fixture().expect("fixture");
        let inspection = inspect_bytes("fixture", "fixture.holo", &bytes).expect("inspect");
        assert!(inspection.footer_verified);
        assert_eq!(inspection.format_version, 3);
        assert!(inspection.directory.is_none());
        assert!(!inspection.directory_embedded);
    }

    #[test]
    fn legacy_application_derives_a_directory_without_requiring_the_extension() {
        let requires = b"legacy capabilities";
        let wasm = b"legacy wasm";
        let manifest = test_manifest(wasm, requires);
        let mut writer = HoloWriter::new();
        writer.set_app_manifest(manifest.canonicalize());
        writer.add_content_blob(address_bytes(requires).as_bytes(), requires);
        writer.add_content_blob(address_bytes(wasm).as_bytes(), wasm);

        let bytes = writer.finish().expect("legacy archive");
        let inspection = inspect_bytes("legacy", "legacy.holo", &bytes).expect("inspect");
        let directory = inspection.directory.expect("derived directory");

        assert!(!inspection.directory_embedded);
        assert_eq!(directory.primary_layer, Some(0));
        assert_eq!(directory.layers.len(), 1);
        assert_eq!(directory.layers[0].kind, "wasm");
        assert_eq!(directory.blobs.len(), 2);
    }

    #[test]
    fn inspection_rejects_a_directory_that_disagrees_with_the_manifest() {
        let requires = b"directory capabilities";
        let wasm = b"directory wasm";
        let manifest = test_manifest(wasm, requires);
        let blobs = [
            (address_bytes(requires), requires.as_slice()),
            (address_bytes(wasm), wasm.as_slice()),
        ];
        let mut directory = holo_directory::derive(
            &manifest,
            blobs
                .iter()
                .map(|(kappa, content)| (kappa.as_bytes(), *content)),
        )
        .expect("directory");
        directory.layers[0].entry = "forged".to_owned();

        let mut writer = HoloWriter::new();
        writer.set_app_manifest(manifest.canonicalize());
        writer.add_extension(
            DIRECTORY_EXTENSION_KEY,
            holo_directory::encode(&directory).expect("encode directory"),
        );
        for (kappa, content) in blobs {
            writer.add_content_blob(kappa.as_bytes(), content);
        }

        let error = inspect_bytes(
            "forged-directory",
            "forged-directory.holo",
            &writer.finish().expect("archive"),
        )
        .expect_err("directory mismatch must fail");
        assert_eq!(error.code(), "LIVE_HOLO_INVALID");
        assert!(error.to_string().contains("does not match"), "{error}");
    }

    #[test]
    fn inspection_rejects_a_content_blob_with_a_forged_kappa() {
        let requires = b"forged capabilities";
        let wasm = b"expected wasm";
        let manifest = test_manifest(wasm, requires);
        let mut writer = HoloWriter::new();
        writer.set_app_manifest(manifest.canonicalize());
        writer.add_content_blob(address_bytes(requires).as_bytes(), requires);
        writer.add_content_blob(address_bytes(wasm).as_bytes(), b"different wasm");

        let error = inspect_bytes(
            "forged-blob",
            "forged-blob.holo",
            &writer.finish().expect("archive"),
        )
        .expect_err("forged content address must fail");
        assert_eq!(error.code(), "LIVE_HOLO_INVALID");
        assert!(error.to_string().contains("does not match its bytes"));
    }

    #[tokio::test]
    async fn run_before_load_is_a_typed_not_found() {
        let runtime = test_runtime("run-before-load");
        let error = runtime
            .run("blake3:test", Vec::new())
            .await
            .expect_err("must fail");
        assert_eq!(error.code(), "LIVE_NOT_FOUND");
    }

    #[tokio::test]
    async fn load_run_unload_round_trip_against_the_wasm_fixture() {
        let runtime = test_runtime("round-trip");
        let kappa = import_fixture(&runtime, "wasm-app");
        let record = runtime.load(&kappa).await.expect("load");
        assert_eq!(record.kappa, kappa);
        assert_eq!(record.input_count, 1);
        assert_eq!(record.output_count, 1);
        assert!(record.resident_bytes > 0);

        let result = runtime
            .run(&kappa, vec![b"hello hologram".to_vec()])
            .await
            .expect("run");
        assert_eq!(result.outputs, vec![b"HELLO HOLOGRAM".to_vec()]);

        let resident = runtime.list().await.expect("list");
        assert_eq!(resident.len(), 1);
        assert_eq!(resident[0].processed, 1);

        runtime.unload(&kappa).await.expect("unload");
        assert!(runtime.list().await.expect("list").is_empty());
        let error = runtime
            .run(&kappa, Vec::new())
            .await
            .expect_err("must fail");
        assert_eq!(error.code(), "LIVE_NOT_FOUND");
        let error = runtime.unload(&kappa).await.expect_err("must fail");
        assert_eq!(error.code(), "LIVE_NOT_FOUND");
    }

    #[tokio::test]
    async fn load_rejects_a_view_only_archive() {
        let runtime = test_runtime("wrong-kind");
        let kappa = import_fixture(&runtime, "view-app");
        let error = runtime.load(&kappa).await.expect_err("must fail");
        assert_eq!(error.code(), "LIVE_CAPABILITY_MISSING");
        assert!(error.to_string().contains("view"), "{error}");
    }

    fn test_runtime(name: &str) -> HoloRuntime {
        let temporary = std::env::temp_dir().join(format!(
            "hologram-live-holo-test-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&temporary);
        let store = Arc::new(ObjectStore::open(temporary).expect("store"));
        HoloRuntime::new(Arc::new(HoloCatalog::new(store)), 8)
    }

    fn import_fixture(runtime: &HoloRuntime, fixture: &str) -> String {
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("features/fixtures")
            .join(fixture)
            .join("hologram.json");
        let compiled = crate::compile::compile_manifest(&manifest).expect("compile fixture");
        runtime
            .catalog
            .import(format!("{fixture}.holo"), compiled.bytes)
            .expect("import fixture")
            .kappa
    }

    fn test_manifest(wasm: &[u8], requires: &[u8]) -> AppManifest {
        AppManifest {
            primary: Some(0),
            requires: address_bytes(requires),
            layers: vec![Layer::wasm(address_bytes(wasm), "holo_run")],
            children: Vec::new(),
        }
    }
}
