use crate::actor::ActorSystem;
use crate::application_plan::{explain_application, ApplicationPlan, PlanLimits};
use crate::audit::{AuditEvent, AuditLog};
use crate::error::{LiveError, Result};
use crate::holo_capability::EffectiveGrant;
use crate::holo_channel::ChannelBroker;
use crate::holo_component::ComponentProvider;
use crate::holo_directory::{self, DIRECTORY_EXTENSION_KEY};
use crate::holo_provider::{
    prepare_and_start_with_admitted_grants, LayerCompletion, LifecycleState, ProviderRegistry,
    ProviderTarget, RunningApplication,
};
use crate::holo_python::PythonRootfsProvider;
use crate::holo_view_provider::ViewProvider;
use crate::holo_wasm::WasmProvider;
use crate::protocol::{
    ApplicationCompletion, HoloInspection, HoloPlan, HoloRunResult, HoloSection, ResidentHolo,
};
use crate::store::ObjectStore;
use crate::util::hex;
use hologram::archive::{HoloLoader, HoloWriter};
use hologram::space::{address_bytes, AppManifest, Realization};
use hologram_view_surface::ViewSurfaceRegistry;
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
        resident_registry(
            Engine::default(),
            None,
            1,
            Some(self.store.clone()),
            Arc::new(ChannelBroker::default()),
        )?
        .evaluate(&mut report);
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
        crate::holo_format::require_current(bytes)?;
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
    crate::holo_format::require_current(bytes)?;
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
    let (directory, application_kappa) = if let Some(manifest_bytes) = plan.app_manifest() {
        let manifest = AppManifest::decode(manifest_bytes).map_err(|error| {
            LiveError::InvalidHolo(format!("decode application manifest: {error:?}"))
        })?;
        let application_kappa = address_bytes(&manifest.canonicalize()).to_string();
        let blobs = plan
            .content_blobs()
            .map_err(|error| LiveError::InvalidHolo(error.to_string()))?;
        let derived = holo_directory::verify_required(
            &manifest,
            directory_extensions.iter().map(|(_, bytes)| *bytes),
            blobs.iter().copied(),
        )?;
        (Some(derived), Some(application_kappa))
    } else {
        if !directory_extensions.is_empty() {
            return Err(LiveError::InvalidHolo(
                "application directory requires an application manifest".to_owned(),
            ));
        }
        (None, None)
    };
    let directory_embedded = directory.is_some();
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
    direct_registry(
        Engine::default(),
        Arc::new(ViewSurfaceRegistry::new()),
        None,
        Arc::new(ChannelBroker::default()),
    )?
    .evaluate(&mut report);
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
    channel_broker: Arc<ChannelBroker>,
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
            channel_broker: Arc::new(ChannelBroker::default()),
        }
    }

    pub fn new_with_grant_and_channel_broker(
        catalog: Arc<HoloCatalog>,
        mailbox_capacity: usize,
        effective_grant: EffectiveGrant,
        channel_broker: Arc<ChannelBroker>,
    ) -> Self {
        let mut runtime = Self::new_with_grant(catalog, mailbox_capacity, effective_grant);
        runtime.channel_broker = channel_broker;
        runtime
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
            Some(self.catalog.store.clone()),
            self.channel_broker.clone(),
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

    /// Load operator-declared resident applications during service startup.
    ///
    /// Every κ goes through the same verified, audited path as an explicit
    /// `holo load` (principal `local-runtime`). Failures are collected, not
    /// propagated: one missing or under-granted declaration must not keep
    /// the other declared applications — or the service itself — down. The
    /// per-entry outcomes let the caller log a startup summary.
    pub async fn load_declared(&self, kappas: &[String]) -> Vec<(String, Result<ResidentHolo>)> {
        let mut outcomes = Vec::with_capacity(kappas.len());
        for kappa in kappas {
            let outcome = self.load_for(kappa, "local-runtime").await;
            match &outcome {
                Ok(_) => {
                    tracing::info!(kappa = %kappa, "declared resident holo application loaded");
                }
                Err(error) => {
                    tracing::error!(kappa = %kappa, error = %error, "declared resident holo application failed to load");
                }
            }
            outcomes.push((kappa.clone(), outcome));
        }
        outcomes
    }

    pub async fn run(&self, kappa: &str, inputs: Vec<Vec<u8>>) -> Result<HoloRunResult> {
        let entry = self.entry(kappa)?.ok_or_else(|| not_resident(kappa))?;
        let outcome = entry.application.invoke(inputs).await?;
        Ok(HoloRunResult {
            kappa: kappa.to_owned(),
            outputs: outcome.outputs,
            elapsed_micros: outcome.elapsed_micros,
            resident_bytes: entry.application.status().resident_bytes,
            completion: public_completion(outcome.completion),
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
    view_surfaces: Arc<ViewSurfaceRegistry>,
    object_store: Option<Arc<ObjectStore>>,
    channel_broker: Arc<ChannelBroker>,
}

/// An explicitly managed direct application lifetime.
///
/// Unlike [`HoloExecutor::execute`], starting a session does not invoke the
/// primary layer or stop providers. Callers may invoke the primary repeatedly
/// (including through a portable View intent) and must call [`Self::stop`] when
/// the user-owned lifetime ends.
pub struct HoloApplicationSession {
    kappa: String,
    application: RunningApplication,
    resident_bytes: usize,
    requested_capabilities_kappa: String,
    effective_grant_kappa: String,
    grant_source: String,
}

impl HoloApplicationSession {
    pub fn archive_kappa(&self) -> &str {
        &self.kappa
    }

    pub fn application_kappa(&self) -> &str {
        &self.application.identity().application_kappa
    }

    pub fn application_kappas(&self) -> Vec<&str> {
        self.application.application_kappas()
    }

    pub fn state(&self) -> LifecycleState {
        self.application.state()
    }

    pub async fn invoke(&self, inputs: Vec<Vec<u8>>) -> Result<HoloRunResult> {
        let outcome = self.application.invoke(inputs).await?;
        Ok(self.result(outcome))
    }

    pub async fn stop(&self) -> Result<()> {
        self.application.stop().await
    }

    async fn invoke_then_stop(&self, inputs: Vec<Vec<u8>>) -> Result<HoloRunResult> {
        let outcome = self.application.invoke_then_stop(inputs).await?;
        Ok(self.result(outcome))
    }

    fn result(&self, outcome: crate::holo_provider::LayerInvocation) -> HoloRunResult {
        HoloRunResult {
            kappa: self.kappa.clone(),
            outputs: outcome.outputs,
            elapsed_micros: outcome.elapsed_micros,
            resident_bytes: self.resident_bytes,
            completion: public_completion(outcome.completion),
            requested_capabilities_kappa: self.requested_capabilities_kappa.clone(),
            effective_grant_kappa: self.effective_grant_kappa.clone(),
            grant_source: self.grant_source.clone(),
            authorization: "allowed".to_owned(),
        }
    }
}

impl HoloExecutor {
    pub fn with_view_surfaces(view_surfaces: Arc<ViewSurfaceRegistry>) -> Self {
        Self {
            engine: Engine::default(),
            view_surfaces,
            object_store: None,
            channel_broker: Arc::new(ChannelBroker::default()),
        }
    }

    pub fn with_object_store(object_store: Arc<ObjectStore>) -> Self {
        Self {
            engine: Engine::default(),
            view_surfaces: Arc::new(ViewSurfaceRegistry::new()),
            object_store: Some(object_store),
            channel_broker: Arc::new(ChannelBroker::default()),
        }
    }

    pub fn with_channel_broker(channel_broker: Arc<ChannelBroker>) -> Self {
        Self {
            engine: Engine::default(),
            view_surfaces: Arc::new(ViewSurfaceRegistry::new()),
            object_store: None,
            channel_broker,
        }
    }

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

    pub async fn start_session(&self, bytes: &[u8]) -> Result<HoloApplicationSession> {
        self.start_session_with_grant(bytes, &EffectiveGrant::local_baseline())
            .await
    }

    pub async fn start_session_with_grant(
        &self,
        bytes: &[u8],
        effective_grant: &EffectiveGrant,
    ) -> Result<HoloApplicationSession> {
        self.start_session_internal(bytes, effective_grant, None)
            .await
    }

    pub async fn start_session_with_grant_and_audit(
        &self,
        bytes: &[u8],
        effective_grant: &EffectiveGrant,
        audit: &AuditLog,
        principal: &str,
    ) -> Result<HoloApplicationSession> {
        self.start_session_internal(bytes, effective_grant, Some((audit, principal)))
            .await
    }

    async fn execute_internal(
        &self,
        bytes: &[u8],
        inputs: Vec<Vec<u8>>,
        effective_grant: &EffectiveGrant,
        audit: Option<(&AuditLog, &str)>,
    ) -> Result<HoloRunResult> {
        self.start_session_internal(bytes, effective_grant, audit)
            .await?
            .invoke_then_stop(inputs)
            .await
    }

    async fn start_session_internal(
        &self,
        bytes: &[u8],
        effective_grant: &EffectiveGrant,
        audit: Option<(&AuditLog, &str)>,
    ) -> Result<HoloApplicationSession> {
        let kappa = format!("blake3:{}", blake3::hash(bytes));
        let mut report = explain_application(bytes, PlanLimits::default(), |_| Ok(None))?;
        let registry = direct_registry(
            self.engine.clone(),
            self.view_surfaces.clone(),
            self.object_store.clone(),
            self.channel_broker.clone(),
        )?;
        registry.evaluate(&mut report);
        let plan = report.into_application_plan()?;
        let requested_capabilities_kappa = plan.requested_capabilities.kappa.clone();
        let admitted_grants = admit_with_audit(&plan, effective_grant, audit).await?;
        let application =
            prepare_and_start_with_admitted_grants(&plan, &registry, &admitted_grants).await?;
        let resident_bytes = application.status().resident_bytes;
        Ok(HoloApplicationSession {
            kappa,
            application,
            resident_bytes,
            requested_capabilities_kappa,
            effective_grant_kappa: effective_grant.kappa.clone(),
            grant_source: effective_grant.source.name().to_owned(),
        })
    }
}

const fn public_completion(completion: LayerCompletion) -> ApplicationCompletion {
    match completion {
        LayerCompletion::Returned => ApplicationCompletion::Returned,
        LayerCompletion::Exited { code } => ApplicationCompletion::Exited { code },
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

fn direct_registry(
    engine: Engine,
    view_surfaces: Arc<ViewSurfaceRegistry>,
    object_store: Option<Arc<ObjectStore>>,
    channel_broker: Arc<ChannelBroker>,
) -> Result<ProviderRegistry> {
    ProviderRegistry::new(
        ProviderTarget::Direct,
        vec![
            Arc::new(WasmProvider::direct(engine)),
            Arc::new(ComponentProvider::direct()),
            Arc::new(ComponentProvider::store_read_direct(object_store.clone())),
            Arc::new(ComponentProvider::store_graph_read_direct(
                object_store.clone(),
            )),
            Arc::new(ComponentProvider::store_write_direct(object_store)),
            Arc::new(ComponentProvider::channel_publish_direct(
                channel_broker.clone(),
            )),
            Arc::new(ComponentProvider::channel_subscribe_direct(channel_broker)),
            Arc::new(ComponentProvider::network_fetch_direct()),
            Arc::new(PythonRootfsProvider),
            Arc::new(ViewProvider::new(view_surfaces)),
        ],
    )
}

fn resident_registry(
    engine: Engine,
    root: Option<kameo::actor::ActorRef<crate::actor::RootSupervisor>>,
    mailbox_capacity: usize,
    object_store: Option<Arc<ObjectStore>>,
    channel_broker: Arc<ChannelBroker>,
) -> Result<ProviderRegistry> {
    ProviderRegistry::new(
        ProviderTarget::Resident,
        vec![
            Arc::new(WasmProvider::resident(engine, root, mailbox_capacity)),
            Arc::new(ComponentProvider::resident()),
            Arc::new(ComponentProvider::store_read_resident(object_store.clone())),
            Arc::new(ComponentProvider::store_graph_read_resident(
                object_store.clone(),
            )),
            Arc::new(ComponentProvider::store_write_resident(object_store)),
            Arc::new(ComponentProvider::channel_publish_resident(
                channel_broker.clone(),
            )),
            Arc::new(ComponentProvider::channel_subscribe_resident(
                channel_broker,
            )),
            Arc::new(ComponentProvider::network_fetch_resident()),
            Arc::new(ViewProvider::headless()),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use hologram::space::{address_bytes, Channel, Layer, Realization};
    use hologram_view_surface::{
        PortableViewAttachment, PortableViewSurface, SurfaceFuture, ViewIntentRequest,
        APPLICATION_INVOKE_INTENT, VIEW_INTENT_VERSION,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct SessionViewSurface {
        attachment: Mutex<Option<PortableViewAttachment>>,
        attached: AtomicUsize,
        detached: AtomicUsize,
    }

    impl PortableViewSurface for SessionViewSurface {
        fn attach(&self, view: PortableViewAttachment) -> SurfaceFuture<'_> {
            Box::pin(async move {
                *self.attachment.lock().expect("attachment") = Some(view);
                self.attached.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
        }

        fn detach<'a>(
            &'a self,
            _id: &'a hologram_view_surface::ViewAttachmentId,
        ) -> SurfaceFuture<'a> {
            Box::pin(async move {
                self.detached.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
        }
    }

    fn wasm_layer(content: hologram::space::KappaLabel71, entry: &str) -> Layer {
        Layer::wasm_with_contract(content, entry, crate::holo_contract::WASM_CONTRACT_CORE_V1)
    }

    fn canonical_capabilities() -> &'static [u8] {
        static CAPABILITIES: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
        CAPABILITIES.get_or_init(crate::holo_capability::empty_canonical)
    }

    fn current_archive(manifest: &AppManifest, contents: &[&[u8]]) -> Vec<u8> {
        let addressed = contents
            .iter()
            .map(|content| (address_bytes(content), *content))
            .collect::<Vec<_>>();
        let directory = holo_directory::derive(
            manifest,
            addressed
                .iter()
                .map(|(kappa, content)| (kappa.as_bytes(), *content)),
        )
        .expect("directory");
        let mut writer = HoloWriter::new();
        writer.set_app_manifest(manifest.canonicalize());
        writer.add_extension(
            DIRECTORY_EXTENSION_KEY,
            holo_directory::encode(&directory).expect("encode directory"),
        );
        for (kappa, content) in addressed {
            writer.add_content_blob(kappa.as_bytes(), content);
        }
        writer.finish().expect("current archive")
    }

    #[test]
    fn provider_completion_stays_distinct_from_output_bytes() {
        assert_eq!(
            public_completion(LayerCompletion::Returned),
            ApplicationCompletion::Returned
        );
        assert_eq!(
            public_completion(LayerCompletion::Exited { code: 23 }),
            ApplicationCompletion::Exited { code: 23 }
        );
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
    fn application_without_a_directory_is_rejected() {
        let requires = canonical_capabilities();
        let wasm = b"wasm";
        let manifest = test_manifest(wasm, requires);
        let mut writer = HoloWriter::new();
        writer.set_app_manifest(manifest.canonicalize());
        writer.add_content_blob(address_bytes(requires).as_bytes(), requires);
        writer.add_content_blob(address_bytes(wasm).as_bytes(), wasm);

        let bytes = writer.finish().expect("archive");
        let error = inspect_bytes("missing-directory", "missing-directory.holo", &bytes)
            .expect_err("application directory is required");
        assert!(error
            .to_string()
            .contains("requires an embedded application directory"));
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
        let expected: [(hologram::space::KappaLabel71, &[u8]); 2] = [
            (address_bytes(requires), requires),
            (address_bytes(wasm), wasm),
        ];
        let directory = holo_directory::derive(
            &manifest,
            expected
                .iter()
                .map(|(kappa, content)| (kappa.as_bytes(), *content)),
        )
        .expect("directory");
        let mut writer = HoloWriter::new();
        writer.set_app_manifest(manifest.canonicalize());
        writer.add_extension(
            DIRECTORY_EXTENSION_KEY,
            holo_directory::encode(&directory).expect("encode directory"),
        );
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
        assert_eq!(result.completion, ApplicationCompletion::Returned);

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
    async fn load_declared_loads_every_valid_kappa_and_skips_failures() {
        let runtime = test_runtime("declared");
        let kappa = import_fixture(&runtime, "wasm-app");
        let missing = format!("blake3:{}", "ef".repeat(32));

        let outcomes = runtime
            .load_declared(&[kappa.clone(), missing.clone()])
            .await;
        assert_eq!(outcomes.len(), 2);
        assert_eq!(outcomes[0].0, kappa);
        assert!(outcomes[0].1.is_ok(), "fixture declaration loads");
        assert_eq!(outcomes[1].0, missing);
        assert!(outcomes[1].1.is_err(), "unknown κ is reported, not fatal");

        let resident = runtime.list().await.expect("list");
        assert_eq!(resident.len(), 1);
        assert_eq!(resident[0].kappa, kappa);

        // A repeated declaration pass is idempotent and still succeeds.
        let repeated = runtime.load_declared(std::slice::from_ref(&kappa)).await;
        assert!(repeated[0].1.is_ok());
        assert_eq!(runtime.list().await.expect("list").len(), 1);

        let result = runtime
            .run(&kappa, vec![b"declared".to_vec()])
            .await
            .expect("declared application runs");
        assert_eq!(result.outputs, vec![b"DECLARED".to_vec()]);
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
        assert_eq!(result.completion, ApplicationCompletion::Returned);
        assert!(result.resident_bytes > 0);
    }

    #[tokio::test]
    async fn explicit_session_keeps_a_portable_view_open_until_stop() {
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("examples/wasm-view/hologram.json");
        let compiled = crate::compile::compile_manifest(&manifest).expect("compile View example");
        let registry = Arc::new(ViewSurfaceRegistry::new());
        let surface = Arc::new(SessionViewSurface::default());
        registry
            .register_portable(surface.clone())
            .expect("register portable surface");

        let session = HoloExecutor::with_view_surfaces(registry)
            .start_session(&compiled.bytes)
            .await
            .expect("start session");
        assert_eq!(session.state(), LifecycleState::Running);
        assert_eq!(surface.attached.load(Ordering::SeqCst), 1);
        assert_eq!(surface.detached.load(Ordering::SeqCst), 0);

        let direct = session
            .invoke(vec![b"direct session".to_vec()])
            .await
            .expect("invoke session");
        assert_eq!(direct.outputs, vec![b"DIRECT SESSION".to_vec()]);
        let attachment = surface
            .attachment
            .lock()
            .expect("attachment")
            .clone()
            .expect("attached View");
        let intent = attachment
            .intents
            .handle(
                &attachment.id,
                ViewIntentRequest {
                    version: VIEW_INTENT_VERSION,
                    name: APPLICATION_INVOKE_INTENT.to_owned(),
                    payload: "view session".to_owned(),
                },
            )
            .await
            .expect("invoke through View");
        assert_eq!(intent.outputs, vec!["VIEW SESSION"]);
        assert_eq!(session.state(), LifecycleState::Running);
        assert_eq!(surface.detached.load(Ordering::SeqCst), 0);

        session.stop().await.expect("stop session");
        session.stop().await.expect("repeated stop is idempotent");
        assert_eq!(session.state(), LifecycleState::Stopped);
        assert_eq!(surface.detached.load(Ordering::SeqCst), 1);
        let error = session
            .invoke(vec![b"after stop".to_vec()])
            .await
            .expect_err("stopped session rejects invocation");
        assert_eq!(error.code(), "LIVE_CONFLICT");
        let error = attachment
            .intents
            .handle(
                &attachment.id,
                ViewIntentRequest {
                    version: VIEW_INTENT_VERSION,
                    name: APPLICATION_INVOKE_INTENT.to_owned(),
                    payload: "stale view".to_owned(),
                },
            )
            .await
            .expect_err("a stale View handler cannot invoke a stopped primary");
        assert!(error.contains("not running"), "{error}");
    }

    #[tokio::test]
    async fn one_shot_executor_runs_component_v1() {
        let archive = component_fixture_archive();
        let result = HoloExecutor::default()
            .execute(&archive, vec![b"component direct".to_vec()])
            .await
            .expect("execute Component v1");
        assert_eq!(result.outputs, vec![b"component direct".to_vec()]);
        assert_eq!(result.completion, ApplicationCompletion::Returned);
        assert!(result.resident_bytes > 0);
    }

    #[tokio::test]
    async fn resident_runtime_runs_component_v1() {
        let runtime = test_runtime("resident-component-v1");
        let archive = component_fixture_archive();
        let kappa = runtime
            .catalog
            .import("component-echo.holo".to_owned(), archive)
            .expect("import component")
            .kappa;
        runtime.load(&kappa).await.expect("load component");
        let result = runtime
            .run(&kappa, vec![b"component resident".to_vec()])
            .await
            .expect("run resident component");
        assert_eq!(result.outputs, vec![b"component resident".to_vec()]);
        assert_eq!(result.completion, ApplicationCompletion::Returned);
        assert_eq!(runtime.list().await.expect("list")[0].processed, 1);
        runtime.unload(&kappa).await.expect("unload component");
    }

    #[tokio::test]
    async fn component_store_read_runs_direct_and_resident_with_the_same_authority() {
        let directory = tempfile::tempdir().expect("tempdir");
        let store = Arc::new(ObjectStore::open(directory.path().join("store")).expect("store"));
        let object = store
            .put("file", "text/plain", None, b"capability mediated")
            .expect("put object");
        let capabilities = storage_capabilities(&object.id);
        let archive = store_read_archive(&capabilities);
        let grant_path = directory.path().join("grant.json");
        std::fs::write(
            &grant_path,
            format!(r#"{{"storage_roots":["{}"]}}"#, object.id),
        )
        .expect("grant");
        let grant = EffectiveGrant::from_development_file(
            &grant_path,
            crate::holo_capability::GrantSource::DirectDevelopmentFile,
        )
        .expect("grant");

        let direct = HoloExecutor::with_object_store(store.clone())
            .execute_with_grant(&archive, vec![object.id.as_bytes().to_vec()], &grant)
            .await
            .expect("direct store.read");
        assert_eq!(direct.outputs, [b"capability mediated"]);

        let catalog = Arc::new(HoloCatalog::new(store));
        let runtime = HoloRuntime::new_with_grant(catalog.clone(), 8, grant);
        let kappa = catalog
            .import("component-store-read.holo".to_owned(), archive)
            .expect("import")
            .kappa;
        runtime.load(&kappa).await.expect("resident load");
        let resident = runtime
            .run(&kappa, vec![object.id.as_bytes().to_vec()])
            .await
            .expect("resident store.read");
        assert_eq!(resident.outputs, [b"capability mediated"]);
        runtime.unload(&kappa).await.expect("unload");
    }

    #[tokio::test]
    async fn component_store_graph_read_resolves_descendants_direct_and_resident() {
        let directory = tempfile::tempdir().expect("tempdir");
        let store = Arc::new(ObjectStore::open(directory.path().join("store")).expect("store"));
        let leaf = store
            .put("file", "text/plain", None, b"graph descendant")
            .expect("put leaf");
        let root_bytes = Channel {
            type_shape: Some(address_bytes(b"graph descendant")),
            decl_payload: b"typed graph root".to_vec(),
        }
        .canonicalize();
        let root = address_bytes(&root_bytes).to_string();
        store
            .cache_addressed(&root, &root_bytes)
            .expect("cache typed root");
        let capabilities = storage_capabilities(&root);
        let archive = store_graph_read_archive(&capabilities);
        let grant_path = directory.path().join("grant.json");
        std::fs::write(&grant_path, format!(r#"{{"storage_roots":["{root}"]}}"#)).expect("grant");
        let grant = EffectiveGrant::from_development_file(
            &grant_path,
            crate::holo_capability::GrantSource::DirectDevelopmentFile,
        )
        .expect("grant");

        let direct = HoloExecutor::with_object_store(store.clone())
            .execute_with_grant(&archive, vec![leaf.id.as_bytes().to_vec()], &grant)
            .await
            .expect("direct graph read");
        assert_eq!(direct.outputs, [b"graph descendant"]);

        let exact_error = HoloExecutor::with_object_store(store.clone())
            .execute_with_grant(
                &store_read_archive(&capabilities),
                vec![leaf.id.as_bytes().to_vec()],
                &grant,
            )
            .await
            .expect_err("exact-root profile must not inherit graph semantics");
        assert!(exact_error.to_string().contains("outside"), "{exact_error}");

        let catalog = Arc::new(HoloCatalog::new(store));
        let runtime = HoloRuntime::new_with_grant(catalog.clone(), 8, grant);
        let kappa = catalog
            .import("component-store-graph-read.holo".to_owned(), archive)
            .expect("import")
            .kappa;
        runtime.load(&kappa).await.expect("resident load");
        let resident = runtime
            .run(&kappa, vec![leaf.id.as_bytes().to_vec()])
            .await
            .expect("resident graph read");
        assert_eq!(resident.outputs, [b"graph descendant"]);
        runtime.unload(&kappa).await.expect("unload");
    }

    #[tokio::test]
    async fn component_store_graph_read_rejects_incomplete_closure_before_instantiation() {
        let directory = tempfile::tempdir().expect("tempdir");
        let store = Arc::new(ObjectStore::open(directory.path().join("store")).expect("store"));
        let missing = address_bytes(b"absent graph member");
        let root_bytes = Channel {
            type_shape: Some(missing),
            decl_payload: Vec::new(),
        }
        .canonicalize();
        let root = address_bytes(&root_bytes).to_string();
        store
            .cache_addressed(&root, &root_bytes)
            .expect("cache typed root");
        let capabilities = storage_capabilities(&root);
        let grant_path = directory.path().join("grant.json");
        std::fs::write(&grant_path, format!(r#"{{"storage_roots":["{root}"]}}"#)).expect("grant");
        let grant = EffectiveGrant::from_development_file(
            &grant_path,
            crate::holo_capability::GrantSource::DirectDevelopmentFile,
        )
        .expect("grant");
        let error = HoloExecutor::with_object_store(store)
            .execute_with_grant(
                &store_graph_read_archive(&capabilities),
                vec![missing.to_string().into_bytes()],
                &grant,
            )
            .await
            .expect_err("incomplete closure");
        assert_eq!(error.code(), "LIVE_CAPABILITY_MISSING");
        assert!(error.to_string().contains("incomplete"), "{error}");
        assert!(!error.to_string().contains(&missing.to_string()), "{error}");
    }

    #[tokio::test]
    async fn component_store_graph_read_child_preserves_root_attenuation() {
        let directory = tempfile::tempdir().expect("tempdir");
        let store = Arc::new(ObjectStore::open(directory.path().join("store")).expect("store"));
        let leaf = store
            .put("file", "text/plain", None, b"child graph leaf")
            .expect("put leaf");
        let root_bytes = Channel {
            type_shape: Some(address_bytes(b"child graph leaf")),
            decl_payload: Vec::new(),
        }
        .canonicalize();
        let root = address_bytes(&root_bytes).to_string();
        store
            .cache_addressed(&root, &root_bytes)
            .expect("cache typed root");
        let capabilities = storage_capabilities(&root);
        let grant_path = directory.path().join("grant.json");
        std::fs::write(&grant_path, format!(r#"{{"storage_roots":["{root}"]}}"#)).expect("grant");
        let grant = EffectiveGrant::from_development_file(
            &grant_path,
            crate::holo_capability::GrantSource::DirectDevelopmentFile,
        )
        .expect("grant");

        let admitted = parent_with_store_graph_read_child(&capabilities, &capabilities);
        let result = HoloExecutor::with_object_store(store.clone())
            .execute_with_grant(&admitted, vec![b"parent".to_vec()], &grant)
            .await
            .expect("attenuated graph child prepares");
        assert_eq!(result.outputs, [b"parent"]);
        assert_eq!(
            store.get(&leaf.id).expect("leaf remains readable"),
            b"child graph leaf"
        );

        let denied = parent_with_store_graph_read_child(
            &capabilities,
            &crate::holo_capability::empty_canonical(),
        );
        let error = HoloExecutor::with_object_store(store)
            .execute_with_grant(&denied, Vec::new(), &grant)
            .await
            .expect_err("empty delegation cannot admit graph root");
        assert_eq!(error.code(), "LIVE_AUTHORIZATION_DENIED");
        assert!(error.to_string().contains("delegated grant"), "{error}");
    }

    #[tokio::test]
    async fn component_store_read_denies_missing_authority_before_instantiation() {
        let directory = tempfile::tempdir().expect("tempdir");
        let store = Arc::new(ObjectStore::open(directory.path()).expect("store"));
        let archive = store_read_archive(canonical_capabilities());
        let error = HoloExecutor::with_object_store(store)
            .execute(&archive, Vec::new())
            .await
            .expect_err("profile requires a storage root");
        assert_eq!(error.code(), "LIVE_AUTHORIZATION_DENIED");
        assert!(error.to_string().contains("storage_roots"), "{error}");
        assert!(error.to_string().contains("hologram:host/store@1.0.0"));
    }

    #[tokio::test]
    async fn component_store_read_child_is_attenuated_to_its_delegated_root() {
        let directory = tempfile::tempdir().expect("tempdir");
        let store = Arc::new(ObjectStore::open(directory.path().join("store")).expect("store"));
        let object = store
            .put("file", "text/plain", None, b"child-readable")
            .expect("put object");
        let capabilities = storage_capabilities(&object.id);
        let grant_path = directory.path().join("grant.json");
        std::fs::write(
            &grant_path,
            format!(r#"{{"storage_roots":["{}"]}}"#, object.id),
        )
        .expect("grant");
        let grant = EffectiveGrant::from_development_file(
            &grant_path,
            crate::holo_capability::GrantSource::DirectDevelopmentFile,
        )
        .expect("grant");

        let admitted = parent_with_store_read_child(&capabilities, &capabilities);
        let result = HoloExecutor::with_object_store(store.clone())
            .execute_with_grant(&admitted, vec![b"parent".to_vec()], &grant)
            .await
            .expect("attenuated child prepares");
        assert_eq!(result.outputs, [b"parent"]);

        let empty = crate::holo_capability::empty_canonical();
        let denied = parent_with_store_read_child(&capabilities, &empty);
        let error = HoloExecutor::with_object_store(store)
            .execute_with_grant(&denied, Vec::new(), &grant)
            .await
            .expect_err("empty delegation cannot admit child root");
        assert_eq!(error.code(), "LIVE_AUTHORIZATION_DENIED");
        assert!(error.to_string().contains("delegated grant"), "{error}");
    }

    #[tokio::test]
    async fn component_store_write_runs_direct_and_resident_with_the_same_authority() {
        let directory = tempfile::tempdir().expect("tempdir");
        let store = Arc::new(ObjectStore::open(directory.path().join("store")).expect("store"));
        let bytes = b"capability mediated write";
        let target = address_bytes(bytes).to_string();
        let quota = u64::try_from(bytes.len()).expect("fixture length");
        let capabilities = storage_write_capabilities(&target, quota);
        let archive = store_write_archive(&capabilities);
        let grant_path = directory.path().join("grant.json");
        std::fs::write(
            &grant_path,
            format!(r#"{{"storage_roots":["{target}"],"storage_quota_bytes":{quota}}}"#),
        )
        .expect("grant");
        let grant = EffectiveGrant::from_development_file(
            &grant_path,
            crate::holo_capability::GrantSource::DirectDevelopmentFile,
        )
        .expect("grant");
        let input = store_write_input(&target, bytes);

        let direct = HoloExecutor::with_object_store(store.clone())
            .execute_with_grant(&archive, vec![input.clone()], &grant)
            .await
            .expect("direct store.write");
        assert_eq!(direct.outputs, [target.as_bytes()]);
        assert_eq!(
            store.get_cached(&target).expect("direct object"),
            Some(bytes.to_vec())
        );

        let catalog = Arc::new(HoloCatalog::new(store.clone()));
        let runtime = HoloRuntime::new_with_grant(catalog.clone(), 8, grant);
        let kappa = catalog
            .import("component-store-write.holo".to_owned(), archive)
            .expect("import")
            .kappa;
        runtime.load(&kappa).await.expect("resident load");
        let resident = runtime
            .run(&kappa, vec![input])
            .await
            .expect("resident store.write");
        assert_eq!(resident.outputs, [target.as_bytes()]);
        assert_eq!(
            store.get_cached(&target).expect("resident object"),
            Some(bytes.to_vec())
        );
        runtime.unload(&kappa).await.expect("unload");
    }

    #[tokio::test]
    async fn component_channels_run_direct_and_resident_on_shared_brokers() {
        let directory = tempfile::tempdir().expect("tempdir");
        let channel = address_bytes(b"runtime channel").to_string();
        let publish_capabilities = channel_capabilities("publish_channels", &channel);
        let subscribe_capabilities = channel_capabilities("subscribe_channels", &channel);
        let publish_archive = channel_archive(true, &publish_capabilities);
        let subscribe_archive = channel_archive(false, &subscribe_capabilities);
        let grant_path = directory.path().join("grant.json");
        std::fs::write(
            &grant_path,
            format!(r#"{{"publish_channels":["{channel}"],"subscribe_channels":["{channel}"]}}"#),
        )
        .expect("grant");
        let grant = EffectiveGrant::from_development_file(
            &grant_path,
            crate::holo_capability::GrantSource::DirectDevelopmentFile,
        )
        .expect("grant");
        let input = channel_publish_input(&channel, b"shared message");

        let direct = HoloExecutor::default();
        direct
            .execute_with_grant(&publish_archive, vec![input.clone()], &grant)
            .await
            .expect("direct publish");
        let received = direct
            .execute_with_grant(
                &subscribe_archive,
                vec![channel.as_bytes().to_vec()],
                &grant,
            )
            .await
            .expect("direct subscribe");
        assert_eq!(received.outputs, [b"shared message"]);

        let store = Arc::new(ObjectStore::open(directory.path().join("store")).expect("store"));
        let catalog = Arc::new(HoloCatalog::new(store));
        let runtime = HoloRuntime::new_with_grant(catalog.clone(), 8, grant);
        let publisher = catalog
            .import("channel-publish.holo".to_owned(), publish_archive)
            .expect("import publisher")
            .kappa;
        let subscriber = catalog
            .import("channel-subscribe.holo".to_owned(), subscribe_archive)
            .expect("import subscriber")
            .kappa;
        runtime.load(&publisher).await.expect("load publisher");
        runtime.load(&subscriber).await.expect("load subscriber");
        runtime
            .run(&publisher, vec![input])
            .await
            .expect("resident publish");
        let received = runtime
            .run(&subscriber, vec![channel.as_bytes().to_vec()])
            .await
            .expect("resident subscribe");
        assert_eq!(received.outputs, [b"shared message"]);
    }

    #[tokio::test]
    async fn component_channel_profiles_deny_missing_authority_before_instantiation() {
        let empty = crate::holo_capability::empty_canonical();
        for (publishes, field, interface) in [
            (true, "publish_channels", "channel-publish"),
            (false, "subscribe_channels", "channel-subscribe"),
        ] {
            let archive = channel_archive(publishes, &empty);
            let error = HoloExecutor::default()
                .execute(&archive, Vec::new())
                .await
                .expect_err("channel profile requires authority");
            assert_eq!(error.code(), "LIVE_AUTHORIZATION_DENIED");
            assert!(error.to_string().contains(field), "{error}");
            assert!(error.to_string().contains(interface), "{error}");
        }
    }

    #[tokio::test]
    async fn component_network_fetch_denies_missing_authority_before_instantiation() {
        let archive = network_fetch_archive(&crate::holo_capability::empty_canonical());
        let error = HoloExecutor::default()
            .execute(&archive, Vec::new())
            .await
            .expect_err("network-fetch profile requires authority");
        assert_eq!(error.code(), "LIVE_AUTHORIZATION_DENIED");
        assert!(
            error.to_string().contains("network_fetch_endpoints"),
            "{error}"
        );
        assert!(error.to_string().contains("network-fetch"), "{error}");
    }

    #[tokio::test]
    async fn component_network_fetch_rejects_private_dns_direct_and_resident() {
        let directory = tempfile::tempdir().expect("tempdir");
        let target = "https://localhost:443/v1";
        let capabilities = network_fetch_capabilities(target);
        let archive = network_fetch_archive(&capabilities);
        let grant_path = directory.path().join("grant.json");
        std::fs::write(
            &grant_path,
            format!(r#"{{"schema_version":2,"network_fetch_endpoints":["{target}"]}}"#),
        )
        .expect("grant");
        let grant = EffectiveGrant::from_development_file(
            &grant_path,
            crate::holo_capability::GrantSource::DirectDevelopmentFile,
        )
        .expect("grant");

        let direct_error = HoloExecutor::default()
            .execute_with_grant(&archive, vec![target.as_bytes().to_vec()], &grant)
            .await
            .expect_err("direct private destination denied");
        assert!(
            direct_error.to_string().contains("forbidden"),
            "{direct_error}"
        );
        assert!(
            !direct_error.to_string().contains("localhost"),
            "{direct_error}"
        );

        let store = Arc::new(ObjectStore::open(directory.path().join("store")).expect("store"));
        let catalog = Arc::new(HoloCatalog::new(store));
        let runtime = HoloRuntime::new_with_grant(catalog.clone(), 8, grant);
        let kappa = catalog
            .import("component-network-fetch.holo".to_owned(), archive)
            .expect("import")
            .kappa;
        runtime.load(&kappa).await.expect("resident load");
        let resident_error = runtime
            .run(&kappa, vec![target.as_bytes().to_vec()])
            .await
            .expect_err("resident private destination denied");
        assert!(
            resident_error.to_string().contains("forbidden"),
            "{resident_error}"
        );
        assert!(
            !resident_error.to_string().contains("localhost"),
            "{resident_error}"
        );
        runtime.unload(&kappa).await.expect("unload");
    }

    #[tokio::test]
    async fn component_channel_children_are_attenuated_to_delegated_sets() {
        let directory = tempfile::tempdir().expect("tempdir");
        let channel = address_bytes(b"delegated channel").to_string();
        let grant_path = directory.path().join("grant.json");
        std::fs::write(
            &grant_path,
            format!(r#"{{"publish_channels":["{channel}"],"subscribe_channels":["{channel}"]}}"#),
        )
        .expect("grant");
        let grant = EffectiveGrant::from_development_file(
            &grant_path,
            crate::holo_capability::GrantSource::DirectDevelopmentFile,
        )
        .expect("grant");

        for (publishes, field) in [(true, "publish_channels"), (false, "subscribe_channels")] {
            let capabilities = channel_capabilities(field, &channel);
            let admitted = parent_with_channel_child(publishes, &capabilities, &capabilities);
            let result = HoloExecutor::default()
                .execute_with_grant(&admitted, vec![b"parent".to_vec()], &grant)
                .await
                .expect("attenuated channel child prepares");
            assert_eq!(result.outputs, [b"parent"]);

            let empty = crate::holo_capability::empty_canonical();
            let denied = parent_with_channel_child(publishes, &capabilities, &empty);
            let error = HoloExecutor::default()
                .execute_with_grant(&denied, Vec::new(), &grant)
                .await
                .expect_err("empty delegation cannot admit child channel");
            assert_eq!(error.code(), "LIVE_AUTHORIZATION_DENIED");
            assert!(error.to_string().contains("delegated grant"), "{error}");
        }
    }

    #[tokio::test]
    async fn component_store_write_denies_missing_roots_or_quota_before_instantiation() {
        let directory = tempfile::tempdir().expect("tempdir");
        let store = Arc::new(ObjectStore::open(directory.path()).expect("store"));
        let bytes = b"never written";
        let target = address_bytes(bytes).to_string();

        let error = HoloExecutor::with_object_store(store.clone())
            .execute(&store_write_archive(canonical_capabilities()), Vec::new())
            .await
            .expect_err("profile requires a storage root");
        assert_eq!(error.code(), "LIVE_AUTHORIZATION_DENIED");
        assert!(error.to_string().contains("storage_roots"), "{error}");
        assert!(error.to_string().contains("store-write@1.0.0"));

        let roots_without_quota = storage_capabilities(&target);
        let grant_path = directory.path().join("grant.json");
        std::fs::write(&grant_path, format!(r#"{{"storage_roots":["{target}"]}}"#)).expect("grant");
        let grant = EffectiveGrant::from_development_file(
            &grant_path,
            crate::holo_capability::GrantSource::DirectDevelopmentFile,
        )
        .expect("grant");
        let error = HoloExecutor::with_object_store(store)
            .execute_with_grant(
                &store_write_archive(&roots_without_quota),
                Vec::new(),
                &grant,
            )
            .await
            .expect_err("profile requires write quota");
        assert_eq!(error.code(), "LIVE_AUTHORIZATION_DENIED");
        assert!(error.to_string().contains("storage_quota_bytes"), "{error}");
    }

    #[tokio::test]
    async fn component_store_write_child_is_attenuated_to_its_delegated_quota() {
        let directory = tempfile::tempdir().expect("tempdir");
        let store = Arc::new(ObjectStore::open(directory.path().join("store")).expect("store"));
        let bytes = b"child-writable";
        let target = address_bytes(bytes).to_string();
        let quota = u64::try_from(bytes.len()).expect("fixture length");
        let capabilities = storage_write_capabilities(&target, quota);
        let grant_path = directory.path().join("grant.json");
        std::fs::write(
            &grant_path,
            format!(r#"{{"storage_roots":["{target}"],"storage_quota_bytes":{quota}}}"#),
        )
        .expect("grant");
        let grant = EffectiveGrant::from_development_file(
            &grant_path,
            crate::holo_capability::GrantSource::DirectDevelopmentFile,
        )
        .expect("grant");

        let admitted = parent_with_store_write_child(&capabilities, &capabilities);
        let result = HoloExecutor::with_object_store(store.clone())
            .execute_with_grant(&admitted, vec![b"parent".to_vec()], &grant)
            .await
            .expect("attenuated child prepares");
        assert_eq!(result.outputs, [b"parent"]);

        let insufficient = storage_write_capabilities(&target, quota - 1);
        let denied = parent_with_store_write_child(&capabilities, &insufficient);
        let error = HoloExecutor::with_object_store(store)
            .execute_with_grant(&denied, Vec::new(), &grant)
            .await
            .expect_err("smaller delegated quota cannot admit child request");
        assert_eq!(error.code(), "LIVE_AUTHORIZATION_DENIED");
        assert!(error.to_string().contains("delegated grant"), "{error}");
    }

    #[tokio::test]
    async fn malformed_component_and_wrong_entry_fail_closed() {
        let malformed = component_archive(b"not a WebAssembly component", "run");
        let error = HoloExecutor::default()
            .execute(&malformed, vec![b"input".to_vec()])
            .await
            .expect_err("malformed component");
        assert_eq!(error.code(), "LIVE_HOLO_INVALID");

        let component = include_bytes!("../tests/fixtures/component-echo/echo.wat");
        let wrong_entry = component_archive(component, "other");
        let error = HoloExecutor::default()
            .execute(&wrong_entry, vec![b"input".to_vec()])
            .await
            .expect_err("wrong component entry");
        assert_eq!(error.code(), "LIVE_CAPABILITY_MISSING");
        assert!(error.to_string().contains("entry must be \"run\""));
    }

    #[test]
    fn resident_plan_advertises_exact_component_provider() {
        let runtime = test_runtime("component-v1-plan");
        let kappa = runtime
            .catalog
            .import(
                "component-echo.holo".to_owned(),
                component_fixture_archive(),
            )
            .expect("import component")
            .kappa;
        let plan = runtime.catalog.plan(&kappa).expect("component plan");
        assert!(plan.runnable);
        assert_eq!(
            plan.layers[0].contract.as_deref(),
            Some(crate::holo_contract::WASM_CONTRACT_COMPONENT_V1)
        );
        assert_eq!(
            plan.layers[0].provider.name.as_deref(),
            Some("wasmtime-component-resident")
        );
        assert_eq!(plan.layers[0].provider.status, "available");
    }

    #[tokio::test]
    async fn explicit_development_grant_runs_an_authorized_wasm_archive() {
        let wasm_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("features/fixtures/wasm-app/transform.wat");
        let wasm = std::fs::read(wasm_path).expect("fixture wasm");
        let capabilities = crate::holo_capability::compile_source(
            std::path::Path::new("request.json"),
            br#"{"schema_version":2,"network_fetch_endpoints":["https://api.example.com:443/v1"]}"#,
        )
        .expect("network request");
        let manifest = AppManifest {
            primary: Some(0),
            requires: address_bytes(&capabilities),
            layers: vec![wasm_layer(address_bytes(&wasm), "holo_run")],
            children: Vec::new(),
        };
        let archive = current_archive(&manifest, &[capabilities.as_slice(), wasm.as_slice()]);

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

        std::fs::write(
            &grant_path,
            br#"{"schema_version":2,"network_fetch_endpoints":["https://api.example.com:443/v1"]}"#,
        )
        .expect("grant");
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
        assert!(!audit_rows.contains("network_fetch_endpoints"));
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
                wasm_layer(address_bytes(malformed_wasm), "run"),
                Layer::view(address_bytes(missing_view), "portable"),
            ],
            children: Vec::new(),
        };
        let bytes = current_archive(&manifest, &[capabilities, malformed_wasm]);

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
        let wasm = std::fs::read_to_string(wasm_path)
            .expect("read Wasm fixture")
            .replacen(
                "(export \"holo_run\")",
                "(export \"holo_run\") (export \"support\")",
                1,
            )
            .into_bytes();
        let capabilities = canonical_capabilities();
        let manifest = AppManifest {
            primary: Some(1),
            requires: address_bytes(capabilities),
            layers: vec![
                wasm_layer(address_bytes(&wasm), "support"),
                wasm_layer(address_bytes(&wasm), "holo_run"),
            ],
            children: Vec::new(),
        };
        let bytes = current_archive(&manifest, &[capabilities, wasm.as_slice()]);

        let direct = HoloExecutor::default()
            .execute(&bytes, vec![b"direct".to_vec()])
            .await
            .expect("direct multi-layer run");
        assert_eq!(direct.outputs, vec![b"DIRECT".to_vec()]);
        assert_eq!(direct.completion, ApplicationCompletion::Returned);

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
        assert_eq!(resident.completion, ApplicationCompletion::Returned);
        runtime.unload(&kappa).await.expect("unload");
    }

    #[tokio::test]
    async fn missing_manifest_wasm_entry_fails_direct_and_resident_preparation() {
        let wasm_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("features/fixtures/wasm-app/transform.wat");
        let wasm = std::fs::read(wasm_path).expect("read Wasm fixture");
        let capabilities = canonical_capabilities();
        let manifest = AppManifest {
            primary: Some(0),
            requires: address_bytes(capabilities),
            layers: vec![wasm_layer(address_bytes(&wasm), "missing_entry")],
            children: Vec::new(),
        };
        let bytes = current_archive(&manifest, &[capabilities, wasm.as_slice()]);

        let direct = HoloExecutor::default()
            .execute(&bytes, Vec::new())
            .await
            .expect_err("direct preparation must reject the missing entry");
        assert_eq!(direct.code(), "LIVE_PROTOCOL_ERROR");
        assert!(direct.to_string().contains("missing_entry"));

        let runtime = test_runtime("missing-manifest-entry");
        let kappa = runtime
            .catalog
            .import("missing-entry.holo".to_owned(), bytes)
            .expect("import")
            .kappa;
        let resident = runtime
            .load(&kappa)
            .await
            .expect_err("resident preparation must reject the missing entry");
        assert_eq!(resident.code(), "LIVE_PROTOCOL_ERROR");
        assert!(resident.to_string().contains("missing_entry"));
        assert!(runtime.list().await.expect("resident list").is_empty());
    }

    #[tokio::test]
    async fn load_rejects_a_view_only_archive() {
        let runtime = test_runtime("wrong-kind");
        let kappa = import_fixture(&runtime, "view-app");
        let error = runtime.load(&kappa).await.expect_err("must fail");
        assert_eq!(error.code(), "LIVE_CAPABILITY_MISSING");
        assert!(error.to_string().contains("library archive"), "{error}");
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
        let bytes = current_archive(&manifest, &[capabilities, bundle]);

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
        assert!(error.to_string().contains("library archive"), "{error}");
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
        let mut contents = vec![capabilities];
        contents.extend(payloads);
        let bytes = current_archive(&manifest, &contents);

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
            layers: vec![wasm_layer(address_bytes(wasm), "holo_run")],
            children: Vec::new(),
        }
    }

    fn component_fixture_archive() -> Vec<u8> {
        let component = include_bytes!("../tests/fixtures/component-echo/echo.wat");
        component_archive(component, crate::holo_contract::COMPONENT_V1_ENTRY)
    }

    fn component_archive(component: &[u8], entry: &str) -> Vec<u8> {
        let capabilities = crate::holo_capability::empty_canonical();
        let manifest = AppManifest {
            primary: Some(0),
            requires: address_bytes(&capabilities),
            layers: vec![Layer::wasm_with_contract(
                address_bytes(component),
                entry,
                crate::holo_contract::WASM_CONTRACT_COMPONENT_V1,
            )],
            children: Vec::new(),
        };
        current_archive(&manifest, &[capabilities.as_slice(), component])
    }

    fn storage_capabilities(root: &str) -> Vec<u8> {
        crate::holo_capability::compile_source(
            std::path::Path::new("storage-capabilities.json"),
            format!(r#"{{"storage_roots":["{root}"]}}"#).as_bytes(),
        )
        .expect("storage capabilities")
    }

    fn storage_write_capabilities(root: &str, quota: u64) -> Vec<u8> {
        crate::holo_capability::compile_source(
            std::path::Path::new("storage-write-capabilities.json"),
            format!(r#"{{"storage_roots":["{root}"],"storage_quota_bytes":{quota}}}"#).as_bytes(),
        )
        .expect("storage write capabilities")
    }

    fn store_write_input(kappa: &str, bytes: &[u8]) -> Vec<u8> {
        let mut input = kappa.as_bytes().to_vec();
        input.push(b'\n');
        input.extend_from_slice(bytes);
        input
    }

    fn channel_publish_input(channel: &str, message: &[u8]) -> Vec<u8> {
        let mut input = channel.as_bytes().to_vec();
        input.push(b'\n');
        input.extend_from_slice(message);
        input
    }

    fn channel_capabilities(field: &str, channel: &str) -> Vec<u8> {
        crate::holo_capability::compile_source(
            std::path::Path::new("channel-capabilities.json"),
            format!(r#"{{"{field}":["{channel}"]}}"#).as_bytes(),
        )
        .expect("channel capabilities")
    }

    fn channel_archive(publishes: bool, capabilities: &[u8]) -> Vec<u8> {
        let (component, contract): (&[u8], &str) = if publishes {
            (
                include_bytes!("../tests/fixtures/component-channel-publish/channel-publish.wasm"),
                crate::holo_contract::WASM_CONTRACT_COMPONENT_CHANNEL_PUBLISH_V1,
            )
        } else {
            (
                include_bytes!(
                    "../tests/fixtures/component-channel-subscribe/channel-subscribe.wasm"
                ),
                crate::holo_contract::WASM_CONTRACT_COMPONENT_CHANNEL_SUBSCRIBE_V1,
            )
        };
        let manifest = AppManifest {
            primary: Some(0),
            requires: address_bytes(capabilities),
            layers: vec![Layer::wasm_with_contract(
                address_bytes(component),
                crate::holo_contract::COMPONENT_V1_ENTRY,
                contract,
            )],
            children: Vec::new(),
        };
        current_archive(&manifest, &[capabilities, component])
    }

    fn network_fetch_capabilities(target: &str) -> Vec<u8> {
        crate::holo_capability::compile_source(
            std::path::Path::new("network-fetch-capabilities.json"),
            format!(r#"{{"schema_version":2,"network_fetch_endpoints":["{target}"]}}"#).as_bytes(),
        )
        .expect("network fetch capabilities")
    }

    fn network_fetch_archive(capabilities: &[u8]) -> Vec<u8> {
        let component =
            include_bytes!("../tests/fixtures/component-network-fetch/network-fetch.wasm");
        let manifest = AppManifest {
            primary: Some(0),
            requires: address_bytes(capabilities),
            layers: vec![Layer::wasm_with_contract(
                address_bytes(component),
                crate::holo_contract::COMPONENT_V1_ENTRY,
                crate::holo_contract::WASM_CONTRACT_COMPONENT_NETWORK_FETCH_V1,
            )],
            children: Vec::new(),
        };
        current_archive(&manifest, &[capabilities, component])
    }

    fn store_read_archive(capabilities: &[u8]) -> Vec<u8> {
        let component = include_bytes!("../tests/fixtures/component-store-read/store-read.wasm");
        let manifest = AppManifest {
            primary: Some(0),
            requires: address_bytes(capabilities),
            layers: vec![Layer::wasm_with_contract(
                address_bytes(component),
                crate::holo_contract::COMPONENT_V1_ENTRY,
                crate::holo_contract::WASM_CONTRACT_COMPONENT_STORE_READ_V1,
            )],
            children: Vec::new(),
        };
        current_archive(&manifest, &[capabilities, component])
    }

    fn store_graph_read_archive(capabilities: &[u8]) -> Vec<u8> {
        let component = include_bytes!("../tests/fixtures/component-store-read/store-read.wasm");
        let manifest = AppManifest {
            primary: Some(0),
            requires: address_bytes(capabilities),
            layers: vec![Layer::wasm_with_contract(
                address_bytes(component),
                crate::holo_contract::COMPONENT_V1_ENTRY,
                crate::holo_contract::WASM_CONTRACT_COMPONENT_STORE_GRAPH_READ_V1,
            )],
            children: Vec::new(),
        };
        current_archive(&manifest, &[capabilities, component])
    }

    fn store_write_archive(capabilities: &[u8]) -> Vec<u8> {
        let component = include_bytes!("../tests/fixtures/component-store-write/store-write.wasm");
        let manifest = AppManifest {
            primary: Some(0),
            requires: address_bytes(capabilities),
            layers: vec![Layer::wasm_with_contract(
                address_bytes(component),
                crate::holo_contract::COMPONENT_V1_ENTRY,
                crate::holo_contract::WASM_CONTRACT_COMPONENT_STORE_WRITE_V1,
            )],
            children: Vec::new(),
        };
        current_archive(&manifest, &[capabilities, component])
    }

    fn parent_with_store_read_child(child_capabilities: &[u8], delegated: &[u8]) -> Vec<u8> {
        parent_with_store_child(
            child_capabilities,
            delegated,
            crate::holo_contract::WASM_CONTRACT_COMPONENT_STORE_READ_V1,
        )
    }

    fn parent_with_store_graph_read_child(child_capabilities: &[u8], delegated: &[u8]) -> Vec<u8> {
        parent_with_store_child(
            child_capabilities,
            delegated,
            crate::holo_contract::WASM_CONTRACT_COMPONENT_STORE_GRAPH_READ_V1,
        )
    }

    fn parent_with_store_child(
        child_capabilities: &[u8],
        delegated: &[u8],
        contract: &str,
    ) -> Vec<u8> {
        let parent_capabilities = crate::holo_capability::empty_canonical();
        let parent_component = include_bytes!("../tests/fixtures/component-echo/echo.wat");
        let child_component =
            include_bytes!("../tests/fixtures/component-store-read/store-read.wasm");
        let child_manifest = AppManifest {
            primary: Some(0),
            requires: address_bytes(child_capabilities),
            layers: vec![Layer::wasm_with_contract(
                address_bytes(child_component),
                crate::holo_contract::COMPONENT_V1_ENTRY,
                contract,
            )],
            children: Vec::new(),
        }
        .canonicalize();
        let parent_manifest = AppManifest {
            primary: Some(0),
            requires: address_bytes(&parent_capabilities),
            layers: vec![Layer::wasm_with_contract(
                address_bytes(parent_component),
                crate::holo_contract::COMPONENT_V1_ENTRY,
                crate::holo_contract::WASM_CONTRACT_COMPONENT_V1,
            )],
            children: vec![(address_bytes(&child_manifest), address_bytes(delegated))],
        };
        let mut contents: Vec<&[u8]> = vec![
            parent_capabilities.as_slice(),
            parent_component,
            child_manifest.as_slice(),
            child_capabilities,
            child_component,
        ];
        if delegated != child_capabilities && delegated != parent_capabilities.as_slice() {
            contents.push(delegated);
        }
        current_archive(&parent_manifest, &contents)
    }

    fn parent_with_store_write_child(child_capabilities: &[u8], delegated: &[u8]) -> Vec<u8> {
        let parent_capabilities = child_capabilities;
        let parent_component = include_bytes!("../tests/fixtures/component-echo/echo.wat");
        let child_component =
            include_bytes!("../tests/fixtures/component-store-write/store-write.wasm");
        let child_manifest = AppManifest {
            primary: Some(0),
            requires: address_bytes(child_capabilities),
            layers: vec![Layer::wasm_with_contract(
                address_bytes(child_component),
                crate::holo_contract::COMPONENT_V1_ENTRY,
                crate::holo_contract::WASM_CONTRACT_COMPONENT_STORE_WRITE_V1,
            )],
            children: Vec::new(),
        }
        .canonicalize();
        let parent_manifest = AppManifest {
            primary: Some(0),
            requires: address_bytes(parent_capabilities),
            layers: vec![Layer::wasm_with_contract(
                address_bytes(parent_component),
                crate::holo_contract::COMPONENT_V1_ENTRY,
                crate::holo_contract::WASM_CONTRACT_COMPONENT_V1,
            )],
            children: vec![(address_bytes(&child_manifest), address_bytes(delegated))],
        };
        let mut contents: Vec<&[u8]> = vec![
            parent_capabilities,
            parent_component,
            child_manifest.as_slice(),
            child_component,
        ];
        if delegated != child_capabilities {
            contents.push(delegated);
        }
        current_archive(&parent_manifest, &contents)
    }

    fn parent_with_channel_child(
        publishes: bool,
        child_capabilities: &[u8],
        delegated: &[u8],
    ) -> Vec<u8> {
        let parent_capabilities = crate::holo_capability::empty_canonical();
        let parent_component = include_bytes!("../tests/fixtures/component-echo/echo.wat");
        let (child_component, contract): (&[u8], &str) = if publishes {
            (
                include_bytes!("../tests/fixtures/component-channel-publish/channel-publish.wasm"),
                crate::holo_contract::WASM_CONTRACT_COMPONENT_CHANNEL_PUBLISH_V1,
            )
        } else {
            (
                include_bytes!(
                    "../tests/fixtures/component-channel-subscribe/channel-subscribe.wasm"
                ),
                crate::holo_contract::WASM_CONTRACT_COMPONENT_CHANNEL_SUBSCRIBE_V1,
            )
        };
        let child_manifest = AppManifest {
            primary: Some(0),
            requires: address_bytes(child_capabilities),
            layers: vec![Layer::wasm_with_contract(
                address_bytes(child_component),
                crate::holo_contract::COMPONENT_V1_ENTRY,
                contract,
            )],
            children: Vec::new(),
        }
        .canonicalize();
        let parent_manifest = AppManifest {
            primary: Some(0),
            requires: address_bytes(&parent_capabilities),
            layers: vec![Layer::wasm_with_contract(
                address_bytes(parent_component),
                crate::holo_contract::COMPONENT_V1_ENTRY,
                crate::holo_contract::WASM_CONTRACT_COMPONENT_V1,
            )],
            children: vec![(address_bytes(&child_manifest), address_bytes(delegated))],
        };
        let mut contents: Vec<&[u8]> = vec![
            parent_capabilities.as_slice(),
            parent_component,
            child_manifest.as_slice(),
            child_capabilities,
            child_component,
        ];
        if delegated != child_capabilities && delegated != parent_capabilities.as_slice() {
            contents.push(delegated);
        }
        current_archive(&parent_manifest, &contents)
    }
}
