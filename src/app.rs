use crate::actor::ActorSystem;
use crate::audit::{AuditEvent, AuditLog};
use crate::auth::{Authorizer, LocalAuthorizer, Principal};
use crate::chat::ChatService;
use crate::config::AppConfig;
use crate::error::{ApiError, LiveError, Result};
use crate::history::HistoryService;
use crate::holo::{HoloCatalog, HoloRuntime};
use crate::holo_capability::{EffectiveGrant, GrantSource};
use crate::models::ModelCatalog;
use crate::module::{ModuleContext, ModuleRegistry, ModuleRouters};
use crate::nodes::NodeDirectory;
use crate::observability::TracingHandle;
use crate::plugin::PluginRegistry;
use crate::protocol::{
    CapabilityManifest, HealthResponse, ModuleInfo, NodeRecord, RpcRequest, RpcResponse,
    PROTOCOL_VERSION,
};
use crate::registry::RegistryProvider;
use crate::store::ObjectStore;
use axum::Router;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::Notify;

struct AppInner {
    config: AppConfig,
    modules: ModuleRegistry,
    store: Arc<ObjectStore>,
    registry: Arc<dyn RegistryProvider>,
    #[cfg(feature = "oci")]
    oci_store: Option<Arc<crate::oci_store::OciStore>>,
    holo_catalog: Arc<HoloCatalog>,
    holo_runtime: Arc<HoloRuntime>,
    history: Arc<HistoryService>,
    models: Arc<ModelCatalog>,
    chat: ChatService,
    nodes: Arc<NodeDirectory>,
    plugins: PluginRegistry,
    _actor_system: ActorSystem,
    audit: AuditLog,
    tracing: TracingHandle,
    authorizer: Arc<dyn Authorizer>,
    shutdown: Notify,
    shutdown_requested: AtomicBool,
    server_id: String,
    cluster_token: Option<String>,
}

#[derive(Clone)]
pub struct AppState {
    inner: Arc<AppInner>,
}

impl AppState {
    pub async fn build(config: AppConfig, tracing: TracingHandle) -> Result<Self> {
        config.create_directories()?;
        let cluster_token = config
            .cluster
            .advertise_endpoint
            .as_ref()
            .map(|_| crate::cluster::load_or_create_token(&config))
            .transpose()?;
        if let Some(cluster_token) = cluster_token.as_deref() {
            if config.auth_token().as_deref() == Some(cluster_token) {
                return Err(LiveError::Config(
                    "the cluster token must be different from the user authentication token"
                        .to_owned(),
                ));
            }
        }
        let modules = ModuleRegistry::build(&config.modules.enabled)?;
        let store = Arc::new(ObjectStore::open(config.paths.data_dir.join("registry"))?);
        let registry = build_registry(&config, store.clone()).await?;
        #[cfg(feature = "oci")]
        let oci_store = open_oci_store(&config).await?;
        #[cfg(feature = "oci")]
        if let Some(store) = &oci_store {
            if let Some(purging) = upload_purging() {
                purge_uploads_periodically(Arc::downgrade(store), purging);
            }
        }
        let holo_catalog = Arc::new(HoloCatalog::new(store.clone()));
        let actor_system = ActorSystem::start();
        let audit = AuditLog::open(
            config.paths.state_dir.join("audit.jsonl"),
            config.server.actor_mailbox_capacity,
            actor_system.root(),
        )
        .await?;
        let effective_grant = match &config.holo.development_grant {
            Some(path) => {
                let grant = EffectiveGrant::from_development_file(
                    path,
                    GrantSource::ServiceDevelopmentFile,
                )?;
                tracing::warn!(
                    path = %path.display(),
                    effective_grant_kappa = %grant.kappa,
                    "resident holo development grant is enabled"
                );
                grant
            }
            None => EffectiveGrant::local_baseline(),
        };
        let holo_runtime = Arc::new(HoloRuntime::new_with_grant_and_audit(
            holo_catalog.clone(),
            config.server.actor_mailbox_capacity,
            effective_grant,
            audit.clone(),
        ));
        let history = Arc::new(HistoryService::open(config.paths.data_dir.join("history"))?);
        let models = Arc::new(ModelCatalog::open(
            store.clone(),
            config.paths.data_dir.join("models"),
        )?);
        let inference_config = config.inference.clone();
        let engine_models = models.clone();
        let mailbox_capacity = config.server.actor_mailbox_capacity;
        let engine = blocking(move || {
            crate::inference::engine_from_config(&inference_config, engine_models, mailbox_capacity)
        })
        .await?;
        let chat = ChatService::new(history.clone(), engine);
        let nodes = Arc::new(NodeDirectory::open(
            config.paths.data_dir.join("control-plane/nodes.json"),
        )?);
        let server_seed = format!(
            "{}\0{}\0{}",
            config.server.listen,
            config.paths.data_dir.display(),
            config.role.as_str()
        );
        let server_id = format!("blake3:{}", blake3::hash(server_seed.as_bytes()).to_hex());
        let plugins = PluginRegistry::build(
            &config.plugins,
            &config.paths.state_dir,
            config.server.actor_mailbox_capacity,
            actor_system.root(),
        )
        .await?;
        let module_context =
            ModuleContext::new(actor_system.clone(), config.paths.data_dir.clone());
        let state = Self {
            inner: Arc::new(AppInner {
                config,
                modules,
                store,
                registry,
                #[cfg(feature = "oci")]
                oci_store,
                holo_catalog,
                holo_runtime,
                history,
                models,
                chat,
                nodes,
                plugins,
                _actor_system: actor_system,
                audit,
                tracing,
                authorizer: Arc::new(LocalAuthorizer),
                shutdown: Notify::new(),
                shutdown_requested: AtomicBool::new(false),
                server_id,
                cluster_token,
            }),
        };
        state.inner.modules.start(&module_context).await?;
        Ok(state)
    }

    pub fn config(&self) -> &AppConfig {
        &self.inner.config
    }

    pub fn store(&self) -> &Arc<ObjectStore> {
        &self.inner.store
    }

    pub fn registry(&self) -> &Arc<dyn RegistryProvider> {
        &self.inner.registry
    }

    /// The registry volume. `None` unless `dev.hologram.live.oci` is enabled.
    #[cfg(feature = "oci")]
    pub fn oci_store(&self) -> Option<&Arc<crate::oci_store::OciStore>> {
        self.inner.oci_store.as_ref()
    }

    pub fn holo_catalog(&self) -> &Arc<HoloCatalog> {
        &self.inner.holo_catalog
    }

    pub fn holo_runtime(&self) -> &Arc<HoloRuntime> {
        &self.inner.holo_runtime
    }

    pub fn history(&self) -> &Arc<HistoryService> {
        &self.inner.history
    }

    pub fn models(&self) -> &Arc<ModelCatalog> {
        &self.inner.models
    }

    pub fn chat(&self) -> &ChatService {
        &self.inner.chat
    }

    pub fn nodes(&self) -> &Arc<NodeDirectory> {
        &self.inner.nodes
    }

    /// Selects the live owner for mutable distributed state.
    ///
    /// The local node participates only when it has an explicitly advertised
    /// endpoint. That prevents an unrouteable member from being selected for
    /// work that another node must be able to forward to it.
    pub fn mutable_owner(&self, resource: &str) -> Result<Option<NodeRecord>> {
        let mut nodes = self.nodes().list()?;
        if let Some(endpoint) = self.inner.config.cluster.advertise_endpoint.as_ref() {
            let local = self.local_node_record(endpoint.trim_end_matches('/').to_owned());
            nodes.retain(|node| node.node_id != local.node_id);
            nodes.push(local);
        }
        Ok(crate::ownership::owner(resource, &nodes).cloned())
    }

    pub(crate) fn cluster_token(&self) -> Option<&str> {
        self.inner.cluster_token.as_deref()
    }

    pub fn plugins(&self) -> &PluginRegistry {
        &self.inner.plugins
    }

    pub fn audit(&self) -> &AuditLog {
        &self.inner.audit
    }

    pub fn tracing(&self) -> &TracingHandle {
        &self.inner.tracing
    }

    pub fn module_router(&self) -> Router<AppState> {
        self.inner.modules.router()
    }

    pub fn module_routers(&self) -> ModuleRouters {
        self.inner.modules.routers()
    }

    pub(crate) fn module_registry(&self) -> &ModuleRegistry {
        &self.inner.modules
    }

    pub fn module_info(&self) -> Vec<ModuleInfo> {
        self.inner.modules.info()
    }

    pub fn health(&self) -> HealthResponse {
        HealthResponse {
            status: "ready".to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            role: self.inner.config.role.as_str().to_owned(),
            modules_ready: self.inner.modules.info().len(),
        }
    }

    pub fn capability_manifest(&self) -> CapabilityManifest {
        let mut operations = self.inner.modules.operations();
        operations.extend(self.inner.plugins.operations());
        let mut modules = self.inner.modules.info();
        modules.extend(self.inner.plugins.info());
        CapabilityManifest {
            protocol_version: PROTOCOL_VERSION,
            server_version: env!("CARGO_PKG_VERSION").to_owned(),
            server_id: self.inner.server_id.clone(),
            role: self.inner.config.role.as_str().to_owned(),
            operations,
            modules,
            maximum_message_bytes: self
                .inner
                .config
                .server
                .max_rpc_bytes
                .try_into()
                .unwrap_or(u32::MAX),
        }
    }

    pub fn local_node_record(&self, endpoint: String) -> NodeRecord {
        let manifest = self.capability_manifest();
        NodeRecord {
            node_id: manifest.server_id,
            version: manifest.server_version,
            operations: manifest
                .operations
                .into_iter()
                .map(|operation| operation.id)
                .collect(),
            endpoint,
            last_seen_millis: crate::util::now_millis(),
        }
    }

    pub fn request_shutdown(&self) {
        self.inner.shutdown_requested.store(true, Ordering::SeqCst);
        self.inner.shutdown.notify_waiters();
    }

    /// Resolves once shutdown is requested, also when it was requested before
    /// this call: `notify_waiters` wakes only waiters already registered, and
    /// registry mode has two listeners that start waiting at different times.
    pub async fn wait_shutdown(&self) {
        let notified = self.inner.shutdown.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if self.inner.shutdown_requested.load(Ordering::SeqCst) {
            return;
        }
        notified.await;
    }

    pub async fn dispatch(&self, principal: &Principal, request: RpcRequest) -> RpcResponse {
        let operation = request.operation();
        let kind = request.kind();
        // Plugin-provided operations fall through to the plugin registry when
        // no builtin module claims them; both native plugin ops
        // (`plugin.list`, `plugin.call`) are advertised by the system module.
        if !self.inner.modules.supports(operation) && !self.inner.plugins.supports(operation) {
            return RpcResponse::Error(ApiError::from(&LiveError::Capability(format!(
                "operation {operation} is not provided by this server"
            ))));
        }
        if let Err(error) = self.inner.authorizer.authorize(principal, operation, kind) {
            return RpcResponse::Error(ApiError::from(&error));
        }
        let resource = resource_for(&request);
        let response = self.dispatch_authorized(principal, request).await;
        if kind != crate::protocol::OperationKind::Read {
            let outcome = match &response {
                RpcResponse::Error(error) => format!("error:{}", error.code),
                _ => "accepted".to_owned(),
            };
            if let Err(error) = self
                .inner
                .audit
                .record(AuditEvent::new(
                    principal.id.clone(),
                    operation,
                    resource,
                    outcome,
                ))
                .await
            {
                tracing::error!(error = %error, "failed to record audit event");
            }
        }
        response
    }

    async fn dispatch_authorized(&self, principal: &Principal, request: RpcRequest) -> RpcResponse {
        match request {
            RpcRequest::Handshake => RpcResponse::CapabilityManifest(self.capability_manifest()),
            RpcRequest::Health => RpcResponse::Health(self.health()),
            RpcRequest::Shutdown => {
                self.request_shutdown();
                RpcResponse::Accepted
            }
            RpcRequest::ModulesList => RpcResponse::Modules(self.module_info()),
            RpcRequest::TracingGet => match self.inner.tracing.current_filter() {
                Ok(filter) => RpcResponse::TracingFilter(filter),
                Err(error) => RpcResponse::Error(ApiError::from(&error)),
            },
            RpcRequest::TracingSet { filter } => match self.inner.tracing.set_filter(&filter) {
                Ok(()) => RpcResponse::TracingFilter(filter),
                Err(error) => RpcResponse::Error(ApiError::from(&error)),
            },
            RpcRequest::RegistryList => {
                let registry = self.inner.registry.clone();
                RpcResponse::from_result(
                    blocking(move || registry.list_objects(None)).await,
                    RpcResponse::Objects,
                )
            }
            RpcRequest::FilesList => {
                let registry = self.inner.registry.clone();
                RpcResponse::from_result(
                    blocking(move || registry.list_objects(Some("file"))).await,
                    RpcResponse::Objects,
                )
            }
            RpcRequest::RegistrySearch { query } => {
                let registry = self.inner.registry.clone();
                RpcResponse::from_result(
                    blocking(move || registry.search(&query)).await,
                    RpcResponse::ObjectPage,
                )
            }
            RpcRequest::FilesSearch { mut query } => {
                let registry = self.inner.registry.clone();
                // The files surface is the file-kind projection of the object
                // surface, so the kind is fixed here rather than trusted from
                // the caller.
                query.kind = Some("file".to_owned());
                RpcResponse::from_result(
                    blocking(move || registry.search(&query)).await,
                    RpcResponse::ObjectPage,
                )
            }
            RpcRequest::RegistryPut {
                kind,
                media_type,
                filename,
                bytes,
            } => {
                let registry = self.inner.registry.clone();
                RpcResponse::from_result(
                    blocking(move || registry.put_object(kind, media_type, filename, &bytes)).await,
                    RpcResponse::Object,
                )
            }
            RpcRequest::FilesPut {
                media_type,
                filename,
                bytes,
            } => {
                let registry = self.inner.registry.clone();
                RpcResponse::from_result(
                    blocking(move || {
                        registry.put_object("file".to_owned(), media_type, filename, &bytes)
                    })
                    .await,
                    RpcResponse::Object,
                )
            }
            RpcRequest::RegistryGet { id } | RpcRequest::FilesGet { id } => {
                let registry = self.inner.registry.clone();
                RpcResponse::from_result(
                    blocking(move || registry.get_object(&id)).await,
                    RpcResponse::ObjectContent,
                )
            }
            RpcRequest::FilesRename { id, filename } => {
                let registry = self.inner.registry.clone();
                RpcResponse::from_result(
                    blocking(move || registry.rename_file(&id, filename)).await,
                    RpcResponse::Object,
                )
            }
            RpcRequest::HoloImport { name, bytes } => {
                let catalog = self.inner.holo_catalog.clone();
                RpcResponse::from_result(
                    blocking(move || catalog.import(name, bytes)).await,
                    RpcResponse::HoloInspection,
                )
            }
            RpcRequest::HoloList => {
                let catalog = self.inner.holo_catalog.clone();
                RpcResponse::from_result(
                    blocking(move || catalog.list()).await,
                    RpcResponse::HoloList,
                )
            }
            RpcRequest::HoloInspect { kappa } => {
                let catalog = self.inner.holo_catalog.clone();
                RpcResponse::from_result(
                    blocking(move || catalog.inspect(&kappa)).await,
                    RpcResponse::HoloInspection,
                )
            }
            RpcRequest::HoloPlan { kappa } => {
                let catalog = self.inner.holo_catalog.clone();
                RpcResponse::from_result(
                    blocking(move || catalog.plan(&kappa)).await,
                    RpcResponse::HoloPlan,
                )
            }
            RpcRequest::HoloVerify { kappa } => {
                let catalog = self.inner.holo_catalog.clone();
                RpcResponse::from_result(
                    blocking(move || catalog.verify(&kappa)).await,
                    RpcResponse::HoloInspection,
                )
            }
            RpcRequest::HoloRemove { kappa } => {
                let catalog = self.inner.holo_catalog.clone();
                match blocking(move || catalog.remove(&kappa)).await {
                    Ok(()) => RpcResponse::Accepted,
                    Err(error) => RpcResponse::Error(ApiError::from(&error)),
                }
            }
            RpcRequest::HoloLoad { kappa } => RpcResponse::from_result(
                self.inner
                    .holo_runtime
                    .load_for(&kappa, &principal.id)
                    .await,
                |record| RpcResponse::HoloResident(vec![record]),
            ),
            RpcRequest::HoloUnload { kappa } => {
                match self.inner.holo_runtime.unload(&kappa).await {
                    Ok(()) => RpcResponse::Accepted,
                    Err(error) => RpcResponse::Error(ApiError::from(&error)),
                }
            }
            RpcRequest::HoloRun { kappa, inputs } => RpcResponse::from_result(
                self.inner.holo_runtime.run(&kappa, inputs).await,
                RpcResponse::HoloRun,
            ),
            RpcRequest::HoloResident => RpcResponse::from_result(
                self.inner.holo_runtime.list().await,
                RpcResponse::HoloResident,
            ),
            RpcRequest::HistoryCreate { title } => {
                let history = self.inner.history.clone();
                RpcResponse::from_result(
                    blocking(move || history.create(title)).await,
                    RpcResponse::Conversation,
                )
            }
            RpcRequest::HistoryList { include_archived } => {
                let history = self.inner.history.clone();
                RpcResponse::from_result(
                    blocking(move || history.list(include_archived)).await,
                    RpcResponse::Conversations,
                )
            }
            RpcRequest::HistoryGet { id } => {
                let history = self.inner.history.clone();
                RpcResponse::from_result(
                    blocking(move || history.get(&id)).await,
                    RpcResponse::Conversation,
                )
            }
            RpcRequest::HistoryAppend { id, role, content } => {
                let history = self.inner.history.clone();
                RpcResponse::from_result(
                    blocking(move || history.append(&id, role, content)).await,
                    RpcResponse::Conversation,
                )
            }
            RpcRequest::HistoryArchive { id, archived } => {
                let history = self.inner.history.clone();
                RpcResponse::from_result(
                    blocking(move || history.set_archived(&id, archived)).await,
                    RpcResponse::Conversation,
                )
            }
            RpcRequest::HistoryDelete { id } => {
                let history = self.inner.history.clone();
                match blocking(move || history.delete(&id)).await {
                    Ok(()) => RpcResponse::Accepted,
                    Err(error) => RpcResponse::Error(ApiError::from(&error)),
                }
            }
            RpcRequest::ChatSend { id, content } => RpcResponse::from_result(
                self.inner.chat.send(&id, content).await,
                RpcResponse::Conversation,
            ),
            RpcRequest::ModelList => {
                let models = self.inner.models.clone();
                RpcResponse::from_result(blocking(move || models.list()).await, RpcResponse::Models)
            }
            RpcRequest::ModelImport { path } => {
                let models = self.inner.models.clone();
                let path = PathBuf::from(path);
                RpcResponse::from_result(
                    blocking(move || {
                        if path.is_file() {
                            models.import_gguf(&path)
                        } else {
                            models.import(&path)
                        }
                    })
                    .await,
                    RpcResponse::Model,
                )
            }
            RpcRequest::ModelRemove { id } => {
                let models = self.inner.models.clone();
                match blocking(move || models.remove(&id)).await {
                    Ok(()) => RpcResponse::Accepted,
                    Err(error) => RpcResponse::Error(ApiError::from(&error)),
                }
            }
            RpcRequest::NodesList => {
                let nodes = self.inner.nodes.clone();
                RpcResponse::from_result(blocking(move || nodes.list()).await, RpcResponse::Nodes)
            }
            RpcRequest::NodeHeartbeat { node } => {
                let nodes = self.inner.nodes.clone();
                match blocking(move || nodes.heartbeat(node)).await {
                    Ok(()) => RpcResponse::Accepted,
                    Err(error) => RpcResponse::Error(ApiError::from(&error)),
                }
            }
            RpcRequest::PluginList => RpcResponse::Plugins(self.inner.plugins.list().await),
            RpcRequest::PluginCall {
                plugin_id,
                operation,
                payload,
            } => RpcResponse::from_result(
                self.inner
                    .plugins
                    .invoke(&plugin_id, &operation, &payload)
                    .await,
                RpcResponse::PluginResult,
            ),
        }
    }
}

/// Build the configured registry provider on a blocking thread.
///
/// The kappa provider owns a blocking HTTP client, which must not be built or
/// dropped on a runtime thread. The CLI paths already construct it inside
/// `spawn_blocking` (see `kappa_client`); the daemon's own start did not.
async fn build_registry(
    config: &AppConfig,
    store: Arc<ObjectStore>,
) -> Result<Arc<dyn RegistryProvider>> {
    let config = config.clone();
    blocking(move || crate::registry::provider_from_config(&config, store)).await
}

/// `storage.maintenance.uploadpurging` from the registry settings, or the
/// reference's default outside registry mode. `None` when it is off.
#[cfg(feature = "oci")]
fn upload_purging() -> Option<crate::registry_compat::Purging> {
    crate::registry_compat::installed().map_or(
        Some(crate::registry_compat::Purging::DEFAULT),
        crate::registry_compat::RegistrySettings::upload_purging,
    )
}

/// Abort upload sessions untouched for longer than the purge age, a minute
/// after start and then every `interval` (`uploadpurging`; the reference
/// waits a random 0 to 59 minutes first). Without it, a client that opens
/// sessions and walks away fills the disk. `dryrun` names what it would
/// abort and aborts nothing. The task ends when the store is dropped.
#[cfg(feature = "oci")]
fn purge_uploads_periodically(
    store: std::sync::Weak<crate::oci_store::OciStore>,
    purging: crate::registry_compat::Purging,
) {
    let first = tokio::time::Instant::now() + std::time::Duration::from_mins(1);
    let mut tick = tokio::time::interval_at(first, purging.interval);
    tokio::spawn(async move {
        loop {
            tick.tick().await;
            let Some(store) = store.upgrade() else { return };
            let now = crate::oci_store::now_ms();
            if purging.dry_run {
                let expired = tokio::task::spawn_blocking(move || store.expired_uploads(now)).await;
                if let Ok(expired) = expired {
                    if !expired.is_empty() {
                        tracing::info!(count = expired.len(), sessions = ?expired, "upload purge (dry run): would abort");
                    }
                }
                continue;
            }
            match tokio::task::spawn_blocking(move || store.purge_expired_uploads(now)).await {
                Ok(Ok(0)) => {}
                Ok(Ok(purged)) => tracing::info!(purged, "expired upload sessions purged"),
                Ok(Err(error)) => tracing::warn!(%error, "upload purge failed"),
                Err(error) => tracing::warn!(%error, "upload purge task failed"),
            }
        }
    });
}

/// Open the registry volume when the registry module is enabled.
///
/// Both databases block on open, and a volume held by another process must
/// stop the start, not the first request.
#[cfg(feature = "oci")]
async fn open_oci_store(config: &AppConfig) -> Result<Option<Arc<crate::oci_store::OciStore>>> {
    use crate::oci_store::{OciStore, OpenOptions};
    if !config
        .modules
        .enabled
        .iter()
        .any(|id| id == crate::modules::oci::MODULE_ID)
    {
        return Ok(None);
    }
    // The volume is the data directory: its marker, `oci/` and `kappa/` sit
    // beside the other modules' directories. Registry mode points the data
    // directory at the volume root.
    let root = config.paths.data_dir.clone();
    blocking(move || {
        let options = OpenOptions {
            create: true,
            // `uploadpurging.age`; the reference's default is 168 hours.
            upload_max_age: upload_purging()
                .map_or(crate::registry_compat::Purging::DEFAULT.age, |purging| {
                    purging.age
                }),
        };
        OciStore::open(&root, options)
            .map(|store| Some(Arc::new(store)))
            .map_err(|error| {
                LiveError::Config(format!("registry volume {}: {error}", root.display()))
            })
    })
    .await
}

async fn blocking<T, F>(function: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T> + Send + 'static,
{
    tokio::task::spawn_blocking(function)
        .await
        .map_err(|error| LiveError::Conflict(format!("blocking task failed: {error}")))?
}

fn resource_for(request: &RpcRequest) -> Option<String> {
    match request {
        RpcRequest::HoloInspect { kappa }
        | RpcRequest::HoloPlan { kappa }
        | RpcRequest::HoloVerify { kappa }
        | RpcRequest::HoloRemove { kappa }
        | RpcRequest::HoloLoad { kappa }
        | RpcRequest::HoloUnload { kappa }
        | RpcRequest::HoloRun { kappa, .. } => Some(kappa.clone()),
        RpcRequest::RegistryGet { id }
        | RpcRequest::FilesGet { id }
        | RpcRequest::FilesRename { id, .. }
        | RpcRequest::HistoryGet { id }
        | RpcRequest::HistoryAppend { id, .. }
        | RpcRequest::HistoryDelete { id }
        | RpcRequest::ChatSend { id, .. }
        | RpcRequest::ModelRemove { id } => Some(id.clone()),
        RpcRequest::ModelImport { path } => Some(path.clone()),
        RpcRequest::RegistryPut { filename, .. } | RpcRequest::FilesPut { filename, .. } => {
            filename.clone()
        }
        RpcRequest::NodeHeartbeat { node } => Some(node.node_id.clone()),
        RpcRequest::PluginCall { plugin_id, .. } => Some(plugin_id.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `hologram serve` with `registry.provider = "kappa"` builds the provider
    /// from `AppState::build`, which is async. The kappa client is a blocking
    /// reqwest client, and building one on a runtime thread panics
    /// ("Cannot drop a runtime in a context where blocking is not allowed").
    /// An optimized build can win that race and start; a debug build on
    /// Windows panicked on every start.
    #[tokio::test]
    async fn the_kappa_provider_is_built_off_the_async_runtime() {
        let root = std::env::temp_dir().join(format!(
            "hologram-app-registry-{}",
            crate::util::now_millis()
        ));
        let store = Arc::new(ObjectStore::open(&root).expect("open store"));
        let mut config = AppConfig::default();
        config.registry.provider = "kappa".to_owned();
        config.registry.endpoint = "http://127.0.0.1:1".to_owned();

        build_registry(&config, store)
            .await
            .expect("the provider builds without touching the network");
        let _ = std::fs::remove_dir_all(root);
    }
}
