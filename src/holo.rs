use crate::actor::ActorSystem;
use crate::application_plan::{explain_application, ApplicationPlan, PlanLimits};
use crate::audit::{AuditEvent, AuditLog};
use crate::error::{LiveError, Result};
use crate::holo_capability::EffectiveGrant;
use crate::holo_directory::{self, DIRECTORY_EXTENSION_KEY};
use crate::holo_provider::{
    prepare_and_start_with_admitted_grants, ProviderRegistry, ProviderTarget, RunningApplication,
};
use crate::holo_python::PythonRootfsProvider;
use crate::holo_wasm::WasmProvider;
use crate::protocol::{HoloInspection, HoloPlan, HoloRunResult, HoloSection, ResidentHolo};
use crate::store::ObjectStore;
use crate::util::hex;
use hologram::archive::{HoloLoader, HoloWriter};
use hologram::space::{address_bytes, AppManifest, Realization};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use wasmtime::Engine;

const HOLO_MEDIA_TYPE: &str = "application/vnd.hologram.holo";

/// Stable, content-addressed catalog support for `.holo` archives.
///
/// The default Hologram Live build intentionally depends only on the archive
/// surface of the pinned Hologram revision. Execution of primary wasm layers
/// runs in-process through wasmtime (see [`HoloRuntime`] and
/// `crate::holo_wasm`). Python OCI payloads in a rootfs layer have an
/// experimental direct provider; tensor, inference-model, and other rootfs
/// layers remain explicit capability seams.
pub struct HoloCatalog {
    store: Arc<ObjectStore>,
}

impl HoloCatalog {
    pub fn new(store: Arc<ObjectStore>) -> Self {
        Self { store }
    }

    pub fn import(&self, name: String, bytes: Vec<u8>) -> Result<HoloInspection> {
        let inspection = inspect_bytes("pending", &name, &bytes)?;
        self.cache_content_blobs(&bytes)?;
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

    pub fn plan(&self, kappa: &str) -> Result<HoloPlan> {
        let bytes = self.bytes(kappa)?;
        let mut report = explain_application(&bytes, PlanLimits::default(), |content| {
            self.resolve_content(content)
        })?;
        resident_registry(Engine::default(), None, 1)?.evaluate(&mut report);
        Ok(HoloPlan::from_report(&report, "resident"))
    }

    pub fn bytes(&self, kappa: &str) -> Result<Vec<u8>> {
        let metadata = self.store.metadata(kappa)?;
        if metadata.kind != "holo" {
            return Err(LiveError::InvalidHolo(format!(
                "object {kappa} is not cataloged as a .holo archive"
            )));
        }
        if !self.store.verify(kappa)? {
            return Err(LiveError::InvalidHolo(format!(
                "content address verification failed for {kappa}"
            )));
        }
        self.store.get(kappa)
    }

    fn resolve_content(&self, kappa: &str) -> Result<Option<Vec<u8>>> {
        self.store.get_cached(kappa)
    }

    fn cache_content_blobs(&self, bytes: &[u8]) -> Result<()> {
        let loader = HoloLoader::from_bytes(bytes)
            .map_err(|error| LiveError::InvalidHolo(error.to_string()))?;
        let plan = loader
            .into_plan()
            .map_err(|error| LiveError::InvalidHolo(error.to_string()))?;
        for (label, content) in plan
            .content_blobs()
            .map_err(|error| LiveError::InvalidHolo(error.to_string()))?
        {
            let kappa = std::str::from_utf8(label).map_err(|_| {
                LiveError::InvalidHolo("content blob kappa is not valid UTF-8".to_owned())
            })?;
            self.store.cache_addressed(kappa, content)?;
        }
        Ok(())
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
    let (directory, application_kappa) = if let Some(manifest_bytes) = plan.app_manifest() {
        let manifest = AppManifest::decode(manifest_bytes).map_err(|error| {
            LiveError::InvalidHolo(format!("decode application manifest: {error:?}"))
        })?;
        let application_kappa = address_bytes(&manifest.canonicalize()).to_string();
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
        (Some(derived), Some(application_kappa))
    } else {
        if directory_embedded {
            return Err(LiveError::InvalidHolo(
                "application directory requires an application manifest".to_owned(),
            ));
        }
        (None, None)
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
        application_kappa,
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

pub fn plan_bytes(bytes: &[u8]) -> Result<HoloPlan> {
    let mut report = explain_application(bytes, PlanLimits::default(), |_| Ok(None))?;
    direct_registry(Engine::default())?.evaluate(&mut report);
    Ok(HoloPlan::from_report(&report, "direct"))
}

/// In-process execution provider for `.holo` archives.
///
/// Resident v1 execution supports ordered Wasm layers and invokes the declared
/// primary position (see `crate::holo_wasm` for the guest contract). Python OCI
/// rootfs payloads are currently direct-only; tensor, other rootfs, and view
/// layers remain typed `LIVE_CAPABILITY_MISSING` seams. Loading a kappa starts
/// provider-owned actors under the runtime's supervision root; `run` invokes
/// the primary provider and `unload` stops providers in reverse order.
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
    effective_grant: EffectiveGrant,
    audit: Option<AuditLog>,
}

/// Cloning is cheap: the actor reference and counters are shared handles.
#[derive(Clone)]
struct ResidentEntry {
    application: Arc<RunningApplication>,
    input_count: usize,
    output_count: usize,
    requested_capabilities_kappa: String,
    effective_grant_kappa: String,
    grant_source: String,
}

impl ResidentEntry {
    fn record(&self, kappa: &str) -> ResidentHolo {
        let status = self.application.status();
        ResidentHolo {
            kappa: kappa.to_owned(),
            state: self.application.state().name().to_owned(),
            input_count: self.input_count,
            output_count: self.output_count,
            resident_bytes: status.resident_bytes,
            queued: status.queued,
            processed: status.processed,
            requested_capabilities_kappa: self.requested_capabilities_kappa.clone(),
            effective_grant_kappa: self.effective_grant_kappa.clone(),
            grant_source: self.grant_source.clone(),
            authorization: "allowed".to_owned(),
        }
    }
}

impl HoloRuntime {
    pub fn new(catalog: Arc<HoloCatalog>, mailbox_capacity: usize) -> Self {
        Self::new_with_grant(catalog, mailbox_capacity, EffectiveGrant::local_baseline())
    }

    pub fn new_with_grant(
        catalog: Arc<HoloCatalog>,
        mailbox_capacity: usize,
        effective_grant: EffectiveGrant,
    ) -> Self {
        Self {
            catalog,
            mailbox_capacity: mailbox_capacity.max(1),
            engine: Engine::default(),
            actors: OnceLock::new(),
            resident: Mutex::new(HashMap::new()),
            effective_grant,
            audit: None,
        }
    }

    pub fn new_with_grant_and_audit(
        catalog: Arc<HoloCatalog>,
        mailbox_capacity: usize,
        effective_grant: EffectiveGrant,
        audit: AuditLog,
    ) -> Self {
        let mut runtime = Self::new_with_grant(catalog, mailbox_capacity, effective_grant);
        runtime.audit = Some(audit);
        runtime
    }

    pub async fn load(&self, kappa: &str) -> Result<ResidentHolo> {
        self.load_for(kappa, "local-runtime").await
    }

    pub async fn load_for(&self, kappa: &str, principal: &str) -> Result<ResidentHolo> {
        if let Some(record) = self.resident_record(kappa)? {
            return Ok(record);
        }
        let bytes = self.catalog.bytes(kappa)?;
        let mut report = explain_application(&bytes, PlanLimits::default(), |content| {
            self.catalog.resolve_content(content)
        })?;
        let registry = resident_registry(
            self.engine.clone(),
            Some(self.actors.get_or_init(ActorSystem::start).root().clone()),
            self.mailbox_capacity,
        )?;
        registry.evaluate(&mut report);
        let plan = report.into_application_plan()?;
        let requested_capabilities_kappa = plan.requested_capabilities.kappa.clone();
        let admitted_grants = admit_with_audit(
            &plan,
            &self.effective_grant,
            self.audit.as_ref().map(|audit| (audit, principal)),
        )
        .await?;
        let application = Arc::new(
            prepare_and_start_with_admitted_grants(&plan, &registry, &admitted_grants).await?,
        );
        // The v1 manifest carries no I/O arity, so the contract's
        // one-output-per-input shape is reported as 1/1.
        let entry = ResidentEntry {
            application: application.clone(),
            input_count: 1,
            output_count: 1,
            requested_capabilities_kappa,
            effective_grant_kappa: self.effective_grant.kappa.clone(),
            grant_source: self.effective_grant.source.name().to_owned(),
        };
        let (record, duplicate) = match self.lock_resident()?.entry(kappa.to_owned()) {
            std::collections::hash_map::Entry::Occupied(existing) => {
                (existing.get().record(kappa), true)
            }
            std::collections::hash_map::Entry::Vacant(slot) => {
                (slot.insert(entry).record(kappa), false)
            }
        };
        if duplicate {
            application.stop().await?;
        }
        Ok(record)
    }

    pub async fn unload(&self, kappa: &str) -> Result<()> {
        let entry = self.lock_resident()?.remove(kappa);
        match entry {
            Some(entry) => entry.application.stop().await,
            None => Ok(()),
        }
    }

    pub async fn run(&self, kappa: &str, inputs: Vec<Vec<u8>>) -> Result<HoloRunResult> {
        let entry = self.entry(kappa)?.ok_or_else(|| not_resident(kappa))?;
        let outcome = entry.application.invoke(inputs).await?;
        Ok(HoloRunResult {
            kappa: kappa.to_owned(),
            outputs: outcome.outputs,
            elapsed_micros: outcome.elapsed_micros,
            resident_bytes: entry.application.status().resident_bytes,
            requested_capabilities_kappa: entry.requested_capabilities_kappa,
            effective_grant_kappa: entry.effective_grant_kappa,
            grant_source: entry.grant_source,
            authorization: "allowed".to_owned(),
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

/// One-shot execution for a self-contained `.holo` file.
///
/// This is deliberately separate from [`HoloRuntime`]: the runtime manages
/// catalog-backed, warm resident programs, while the executor verifies,
/// compiles, runs, and drops one local archive without a service process.
#[derive(Default)]
pub struct HoloExecutor {
    engine: Engine,
}

impl HoloExecutor {
    pub async fn execute(&self, bytes: &[u8], inputs: Vec<Vec<u8>>) -> Result<HoloRunResult> {
        self.execute_with_grant(bytes, inputs, &EffectiveGrant::local_baseline())
            .await
    }

    pub async fn execute_with_grant(
        &self,
        bytes: &[u8],
        inputs: Vec<Vec<u8>>,
        effective_grant: &EffectiveGrant,
    ) -> Result<HoloRunResult> {
        self.execute_internal(bytes, inputs, effective_grant, None)
            .await
    }

    pub async fn execute_with_grant_and_audit(
        &self,
        bytes: &[u8],
        inputs: Vec<Vec<u8>>,
        effective_grant: &EffectiveGrant,
        audit: &AuditLog,
        principal: &str,
    ) -> Result<HoloRunResult> {
        self.execute_internal(bytes, inputs, effective_grant, Some((audit, principal)))
            .await
    }

    async fn execute_internal(
        &self,
        bytes: &[u8],
        inputs: Vec<Vec<u8>>,
        effective_grant: &EffectiveGrant,
        audit: Option<(&AuditLog, &str)>,
    ) -> Result<HoloRunResult> {
        let kappa = format!("blake3:{}", blake3::hash(bytes));
        let mut report = explain_application(bytes, PlanLimits::default(), |_| Ok(None))?;
        let registry = direct_registry(self.engine.clone())?;
        registry.evaluate(&mut report);
        let plan = report.into_application_plan()?;
        let requested_capabilities_kappa = plan.requested_capabilities.kappa.clone();
        let admitted_grants = admit_with_audit(&plan, effective_grant, audit).await?;
        let application =
            prepare_and_start_with_admitted_grants(&plan, &registry, &admitted_grants).await?;
        let resident_bytes = application.status().resident_bytes;
        let outcome = application.invoke_then_stop(inputs).await?;
        Ok(HoloRunResult {
            kappa,
            outputs: outcome.outputs,
            elapsed_micros: outcome.elapsed_micros,
            resident_bytes,
            requested_capabilities_kappa,
            effective_grant_kappa: effective_grant.kappa.clone(),
            grant_source: effective_grant.source.name().to_owned(),
            authorization: "allowed".to_owned(),
        })
    }
}

async fn admit_with_audit(
    plan: &ApplicationPlan,
    effective_grant: &EffectiveGrant,
    audit: Option<(&AuditLog, &str)>,
) -> Result<HashMap<usize, EffectiveGrant>> {
    let mut decisions = Vec::new();
    let admission = plan.admitted_grants_with(effective_grant, |decision| decisions.push(decision));
    let audit_result = if let Some((audit, principal)) = audit {
        let mut result = Ok(());
        for decision in decisions {
            if let Err(error) = audit
                .record(AuditEvent::capability_decision(principal, decision))
                .await
            {
                result = Err(error);
                break;
            }
        }
        if result.is_ok() {
            result = audit.flush().await;
        }
        result
    } else {
        Ok(())
    };

    match (admission, audit_result) {
        (Err(error), audit) => {
            if let Err(audit_error) = audit {
                tracing::error!(error = %audit_error, "failed to persist denied capability decision");
            }
            Err(error)
        }
        (Ok(_), Err(error)) => Err(error),
        (Ok(grants), Ok(())) => Ok(grants),
    }
}

fn not_resident(kappa: &str) -> LiveError {
    LiveError::NotFound(format!(
        "{kappa} is not loaded as a resident holo; run `hologram holo load {kappa}` first"
    ))
}

fn direct_registry(engine: Engine) -> Result<ProviderRegistry> {
    ProviderRegistry::new(
        ProviderTarget::Direct,
        vec![
            Arc::new(WasmProvider::direct(engine)),
            Arc::new(PythonRootfsProvider),
        ],
    )
}

fn resident_registry(
    engine: Engine,
    root: Option<kameo::actor::ActorRef<crate::actor::RootSupervisor>>,
    mailbox_capacity: usize,
) -> Result<ProviderRegistry> {
    ProviderRegistry::new(
        ProviderTarget::Resident,
        vec![Arc::new(WasmProvider::resident(
            engine,
            root,
            mailbox_capacity,
        ))],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use hologram::space::{address_bytes, Layer, Realization};

    fn canonical_capabilities() -> &'static [u8] {
        static CAPABILITIES: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
        CAPABILITIES.get_or_init(crate::holo_capability::empty_canonical)
    }

    #[test]
    fn fixture_is_a_valid_holo_archive() {
        let bytes = HoloCatalog::fixture().expect("fixture");
        let inspection = inspect_bytes("fixture", "fixture.holo", &bytes).expect("inspect");
        assert!(inspection.footer_verified);
        assert_eq!(inspection.format_version, 4);
        assert!(inspection.application_kappa.is_none());
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
        assert_eq!(
            inspection.application_kappa.as_deref(),
            Some(address_bytes(&manifest.canonicalize()).to_string().as_str())
        );
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
        assert_eq!(record.state, "running");
        assert_eq!(record.input_count, 1);
        assert_eq!(record.output_count, 1);
        assert!(record.resident_bytes > 0);
        let repeated = runtime.load(&kappa).await.expect("idempotent load");
        assert_eq!(repeated.kappa, kappa);
        assert_eq!(runtime.list().await.expect("list").len(), 1);

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
        runtime
            .unload(&kappa)
            .await
            .expect("repeated unload is idempotent");
    }

    #[tokio::test]
    async fn one_shot_executor_runs_a_self_contained_archive() {
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("features/fixtures/wasm-app/hologram.json");
        let compiled = crate::compile::compile_manifest(&manifest).expect("compile fixture");
        let result = HoloExecutor::default()
            .execute(&compiled.bytes, vec![b"hello holo".to_vec()])
            .await
            .expect("execute");
        assert_eq!(result.outputs, vec![b"HELLO HOLO".to_vec()]);
        assert!(result.resident_bytes > 0);
    }

    #[tokio::test]
    async fn explicit_development_grant_runs_an_authorized_wasm_archive() {
        let wasm_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("features/fixtures/wasm-app/transform.wat");
        let wasm = std::fs::read(wasm_path).expect("fixture wasm");
        let capabilities = crate::holo_capability::compile_source(
            std::path::Path::new("request.json"),
            br#"{"network_fetch":true}"#,
        )
        .expect("network request");
        let manifest = AppManifest {
            primary: Some(0),
            requires: address_bytes(&capabilities),
            layers: vec![Layer::wasm(address_bytes(&wasm), "holo_run")],
            children: Vec::new(),
        };
        let mut writer = HoloWriter::new();
        writer.set_app_manifest(manifest.canonicalize());
        writer.add_content_blob(
            address_bytes(&capabilities).as_bytes(),
            capabilities.as_slice(),
        );
        writer.add_content_blob(address_bytes(&wasm).as_bytes(), wasm.as_slice());
        let archive = writer.finish().expect("archive");

        let directory = tempfile::tempdir().expect("tempdir");
        let audit_path = directory.path().join("audit.jsonl");
        let actors = ActorSystem::start();
        let audit = AuditLog::open(&audit_path, 8, actors.root())
            .await
            .expect("audit");

        let error = HoloExecutor::default()
            .execute_with_grant_and_audit(
                &archive,
                vec![b"denied".to_vec()],
                &EffectiveGrant::local_baseline(),
                &audit,
                "local-cli",
            )
            .await
            .expect_err("baseline denies network request");
        assert_eq!(error.code(), "LIVE_AUTHORIZATION_DENIED");

        let grant_path = directory.path().join("grant.json");
        std::fs::write(&grant_path, b"{}").expect("insufficient grant");
        let insufficient_grant = EffectiveGrant::from_development_file(
            &grant_path,
            crate::holo_capability::GrantSource::DirectDevelopmentFile,
        )
        .expect("insufficient grant");
        let error = HoloExecutor::default()
            .execute_with_grant_and_audit(
                &archive,
                vec![b"still denied".to_vec()],
                &insufficient_grant,
                &audit,
                "local-cli",
            )
            .await
            .expect_err("an explicit but insufficient grant must remain denied");
        assert_eq!(error.code(), "LIVE_AUTHORIZATION_DENIED");

        std::fs::write(&grant_path, br#"{"network_fetch":true}"#).expect("grant");
        let grant = EffectiveGrant::from_development_file(
            &grant_path,
            crate::holo_capability::GrantSource::DirectDevelopmentFile,
        )
        .expect("grant");
        let result = HoloExecutor::default()
            .execute_with_grant_and_audit(
                &archive,
                vec![b"authorized".to_vec()],
                &grant,
                &audit,
                "local-cli",
            )
            .await
            .expect("authorized execution");
        assert_eq!(result.outputs, vec![b"AUTHORIZED".to_vec()]);
        assert_eq!(
            result.requested_capabilities_kappa,
            address_bytes(&capabilities).to_string()
        );
        assert_eq!(result.effective_grant_kappa, grant.kappa);
        assert_eq!(result.grant_source, "direct_development_file");
        assert_eq!(result.authorization, "allowed");

        let audit_rows = std::fs::read_to_string(audit_path).expect("audit rows");
        let rows = audit_rows
            .lines()
            .map(|row| serde_json::from_str::<serde_json::Value>(row).expect("audit JSON"))
            .collect::<Vec<_>>();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0]["outcome"], "denied");
        assert_eq!(rows[1]["outcome"], "denied");
        assert_eq!(rows[2]["outcome"], "allowed");
        assert!(rows.iter().all(|row| row["principal"] == "local-cli"));
        assert!(!audit_rows.contains("still denied"));
        assert!(!audit_rows.contains("authorized"));
        assert!(!audit_rows.contains("network_fetch"));
    }

    #[tokio::test]
    async fn one_shot_executor_explains_that_thin_content_is_unavailable() {
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("features/fixtures/wasm-app/hologram.json");
        let compiled =
            crate::compile::compile_manifest_with(&manifest, crate::compile::HoloPackaging::Thin)
                .expect("compile fixture");
        let error = HoloExecutor::default()
            .execute(&compiled.bytes, Vec::new())
            .await
            .expect_err("thin local execution needs a content store");
        assert_eq!(error.code(), "LIVE_NOT_FOUND");
        assert!(error.to_string().contains("cannot resolve"), "{error}");
        assert!(
            error.to_string().contains("required capabilities"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn missing_non_primary_content_blocks_primary_provider_execution() {
        let capabilities = canonical_capabilities();
        let malformed_wasm = b"this is not a wasm module";
        let missing_view = b"missing view";
        let manifest = AppManifest {
            primary: Some(0),
            requires: address_bytes(capabilities),
            layers: vec![
                Layer::wasm(address_bytes(malformed_wasm), "run"),
                Layer::view(address_bytes(missing_view), "portable"),
            ],
            children: Vec::new(),
        };
        let mut writer = HoloWriter::new();
        writer.set_app_manifest(manifest.canonicalize());
        writer.add_content_blob(address_bytes(capabilities).as_bytes(), capabilities);
        writer.add_content_blob(address_bytes(malformed_wasm).as_bytes(), malformed_wasm);
        let bytes = writer.finish().expect("partial archive");

        let error = HoloExecutor::default()
            .execute(&bytes, Vec::new())
            .await
            .expect_err("missing secondary layer must fail before compiling primary wasm");

        assert_eq!(error.code(), "LIVE_NOT_FOUND");
        assert!(error.to_string().contains("layer 1"), "{error}");
        assert!(
            error
                .to_string()
                .contains(&address_bytes(missing_view).to_string()),
            "{error}"
        );
    }

    #[tokio::test]
    async fn resident_runtime_resolves_a_thin_archive_from_the_content_cache() {
        let runtime = test_runtime("thin-resolution");
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("features/fixtures/wasm-app/hologram.json");
        let fat =
            crate::compile::compile_manifest_with(&manifest, crate::compile::HoloPackaging::Fat)
                .expect("fat");
        runtime
            .catalog
            .import("fat.holo".to_owned(), fat.bytes)
            .expect("import fat");
        let thin =
            crate::compile::compile_manifest_with(&manifest, crate::compile::HoloPackaging::Thin)
                .expect("thin");
        let thin_kappa = runtime
            .catalog
            .import("thin.holo".to_owned(), thin.bytes)
            .expect("import thin")
            .kappa;

        let plan = runtime.catalog.plan(&thin_kappa).expect("plan thin");
        assert_eq!(plan.packaging, "thin");
        assert_eq!(plan.execution_target, "resident");
        assert_eq!(
            plan.capabilities.resolution_source.as_deref(),
            Some("local_store")
        );
        assert_eq!(
            plan.layers[0].resolution_source.as_deref(),
            Some("local_store")
        );
        assert_eq!(plan.layers[0].provider.status, "available");
        assert!(plan.runnable);

        runtime.load(&thin_kappa).await.expect("load thin");
        let result = runtime
            .run(&thin_kappa, vec![b"resolved".to_vec()])
            .await
            .expect("run thin");
        assert_eq!(result.outputs, vec![b"RESOLVED".to_vec()]);
    }

    #[tokio::test]
    async fn direct_and_resident_execution_support_a_nonzero_primary_in_wasm_layers() {
        let wasm_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("features/fixtures/wasm-app/transform.wat");
        let wasm = std::fs::read(wasm_path).expect("read Wasm fixture");
        let capabilities = canonical_capabilities();
        let manifest = AppManifest {
            primary: Some(1),
            requires: address_bytes(capabilities),
            layers: vec![
                Layer::wasm(address_bytes(&wasm), "support"),
                Layer::wasm(address_bytes(&wasm), "holo_run"),
            ],
            children: Vec::new(),
        };
        let mut writer = HoloWriter::new();
        writer.set_app_manifest(manifest.canonicalize());
        writer.add_content_blob(address_bytes(capabilities).as_bytes(), capabilities);
        writer.add_content_blob(address_bytes(&wasm).as_bytes(), wasm.as_slice());
        let bytes = writer.finish().expect("multi-layer Wasm archive");

        let direct = HoloExecutor::default()
            .execute(&bytes, vec![b"direct".to_vec()])
            .await
            .expect("direct multi-layer run");
        assert_eq!(direct.outputs, vec![b"DIRECT".to_vec()]);

        let runtime = test_runtime("multi-layer-primary");
        let kappa = runtime
            .catalog
            .import("multi-wasm.holo".to_owned(), bytes)
            .expect("import")
            .kappa;
        runtime
            .load(&kappa)
            .await
            .expect("resident multi-layer load");
        let resident = runtime
            .run(&kappa, vec![b"resident".to_vec()])
            .await
            .expect("resident multi-layer run");
        assert_eq!(resident.outputs, vec![b"RESIDENT".to_vec()]);
        runtime.unload(&kappa).await.expect("unload");
    }

    #[tokio::test]
    async fn load_rejects_a_view_only_archive() {
        let runtime = test_runtime("wrong-kind");
        let kappa = import_fixture(&runtime, "view-app");
        let error = runtime.load(&kappa).await.expect_err("must fail");
        assert_eq!(error.code(), "LIVE_CAPABILITY_MISSING");
        assert!(error.to_string().contains("view"), "{error}");
    }

    #[tokio::test]
    async fn model_only_execution_reports_the_missing_inference_provider() {
        let bundle = b"deterministic model bundle";
        let capabilities = canonical_capabilities();
        let manifest = AppManifest {
            primary: None,
            requires: address_bytes(capabilities),
            layers: vec![Layer::inference_model(
                address_bytes(bundle),
                "ai.default",
                "uor-r4",
            )],
            children: Vec::new(),
        };
        let mut writer = HoloWriter::new();
        writer.set_app_manifest(manifest.canonicalize());
        writer.add_content_blob(address_bytes(capabilities).as_bytes(), capabilities);
        writer.add_content_blob(address_bytes(bundle).as_bytes(), bundle);
        let bytes = writer.finish().expect("model archive");

        let plan = plan_bytes(&bytes).expect("model plan remains inspectable");
        assert!(!plan.runnable);
        assert_eq!(plan.layers[0].kind, "inference-model");
        assert_eq!(plan.layers[0].provider.status, "unavailable");
        assert!(plan
            .blockers
            .iter()
            .any(|blocker| blocker.kind == "provider_unavailable"
                && blocker.error_code == "LIVE_CAPABILITY_MISSING"));

        let error = HoloExecutor::default()
            .execute(&bytes, Vec::new())
            .await
            .expect_err("model provider is not connected");
        assert_eq!(error.code(), "LIVE_CAPABILITY_MISSING");
        assert!(error.to_string().contains("ai.default (uor-r4)"), "{error}");
        assert!(error.to_string().contains("inference provider"), "{error}");
    }

    #[test]
    fn unsupported_layer_kinds_remain_visible_in_a_blocked_plan() {
        let capabilities = canonical_capabilities();
        let payloads: [&[u8]; 4] = [b"view", b"tensor", b"rootfs", b"model"];
        let manifest = AppManifest {
            primary: Some(2),
            requires: address_bytes(capabilities),
            layers: vec![
                Layer::view(address_bytes(payloads[0]), "portable"),
                Layer::tensor(address_bytes(payloads[1]), "session"),
                Layer::rootfs(address_bytes(payloads[2]), "boot", "aarch64"),
                Layer::inference_model(address_bytes(payloads[3]), "ai.default", "uor-r4"),
            ],
            children: Vec::new(),
        };
        let mut writer = HoloWriter::new();
        writer.set_app_manifest(manifest.canonicalize());
        writer.add_content_blob(address_bytes(capabilities).as_bytes(), capabilities);
        for payload in payloads {
            writer.add_content_blob(address_bytes(payload).as_bytes(), payload);
        }
        let bytes = writer.finish().expect("multi-provider archive");

        let plan = plan_bytes(&bytes).expect("unsupported providers are explanatory");

        assert!(!plan.runnable);
        assert_eq!(
            plan.layers
                .iter()
                .map(|layer| layer.kind.as_str())
                .collect::<Vec<_>>(),
            vec!["view", "tensor", "rootfs", "inference-model"]
        );
        assert!(plan
            .layers
            .iter()
            .all(|layer| layer.provider.status == "unavailable"));
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
