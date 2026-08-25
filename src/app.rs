use crate::actor::ActorSystem;
use crate::audit::{AuditEvent, AuditLog};
use crate::auth::{Authorizer, LocalAuthorizer, Principal};
use crate::chat::ChatService;
use crate::config::AppConfig;
use crate::error::{ApiError, LiveError, Result};
use crate::history::HistoryService;
use crate::holo::{HoloCatalog, HoloRuntime};
use crate::models::ModelCatalog;
use crate::module::{ModuleContext, ModuleRegistry};
use crate::nodes::NodeDirectory;
use crate::observability::TracingHandle;
use crate::plugin::PluginRegistry;
use crate::protocol::{
    CapabilityManifest, HealthResponse, ModuleInfo, RpcRequest, RpcResponse, PROTOCOL_VERSION,
};
use crate::registry::{LocalRegistryProvider, RegistryProvider};
use crate::store::ObjectStore;
use axum::Router;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Notify;

struct AppInner {
    config: AppConfig,
    modules: ModuleRegistry,
    store: Arc<ObjectStore>,
    registry: Arc<dyn RegistryProvider>,
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
    server_id: String,
}

#[derive(Clone)]
pub struct AppState {
    inner: Arc<AppInner>,
}

impl AppState {
    pub async fn build(config: AppConfig, tracing: TracingHandle) -> Result<Self> {
        config.create_directories()?;
        let modules = ModuleRegistry::build(&config.modules.enabled)?;
        let store = Arc::new(ObjectStore::open(config.paths.data_dir.join("registry"))?);
        let registry: Arc<dyn RegistryProvider> =
            Arc::new(LocalRegistryProvider::new(store.clone()));
        let holo_catalog = Arc::new(HoloCatalog::new(store.clone()));
        let holo_runtime = Arc::new(HoloRuntime::new(
            holo_catalog.clone(),
            config.server.actor_mailbox_capacity,
        ));
        let history = Arc::new(HistoryService::open(config.paths.data_dir.join("history"))?);
        let models = Arc::new(ModelCatalog::open(
            store.clone(),
            config.paths.data_dir.join("models"),
        )?);
        let engine = crate::inference::engine_from_config(
            &config.inference,
            models.clone(),
            config.server.actor_mailbox_capacity,
        )?;
        let chat = ChatService::new(history.clone(), engine);
        let nodes = Arc::new(NodeDirectory::open(
            config.paths.data_dir.join("control-plane/nodes.json"),
        )?);
        let actor_system = ActorSystem::start();
        let audit = AuditLog::open(
            config.paths.state_dir.join("audit.jsonl"),
            config.server.actor_mailbox_capacity,
            actor_system.root(),
        )
        .await?;
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
                server_id,
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

    pub fn request_shutdown(&self) {
        self.inner.shutdown.notify_waiters();
    }

    pub async fn wait_shutdown(&self) {
        self.inner.shutdown.notified().await;
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
        let response = self.dispatch_authorized(request).await;
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

    async fn dispatch_authorized(&self, request: RpcRequest) -> RpcResponse {
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
            RpcRequest::HoloLoad { kappa } => {
                RpcResponse::from_result(self.inner.holo_runtime.load(&kappa).await, |record| {
                    RpcResponse::HoloResident(vec![record])
                })
            }
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
                RpcResponse::from_result(
                    blocking(move || models.import(&PathBuf::from(path))).await,
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
