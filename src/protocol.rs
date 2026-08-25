use crate::application_plan::{
    layer_kind_name, ApplicationPlanReport, ProviderAvailability, ResolutionSource,
};
use crate::error::{ApiError, LiveError};
use hologram::space::LayerKind;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

pub use crate::application_plan::HoloIdentity;

pub const PROTOCOL_VERSION: u16 = 1;

pub mod operation {
    pub const SYSTEM_HANDSHAKE: &str = "system.handshake";
    pub const SYSTEM_HEALTH: &str = "system.health";
    pub const SYSTEM_SHUTDOWN: &str = "system.shutdown";
    pub const MODULES_LIST: &str = "modules.list";
    pub const TRACING_GET: &str = "tracing.get";
    pub const TRACING_SET: &str = "tracing.set";
    pub const REGISTRY_LIST: &str = "registry.list";
    pub const REGISTRY_PUT: &str = "registry.put";
    pub const REGISTRY_GET: &str = "registry.get";
    pub const FILES_LIST: &str = "files.list";
    pub const FILES_PUT: &str = "files.put";
    pub const FILES_GET: &str = "files.get";
    pub const FILES_RENAME: &str = "files.rename";
    pub const HOLO_IMPORT: &str = "holo.import";
    pub const HOLO_LIST: &str = "holo.list";
    pub const HOLO_INSPECT: &str = "holo.inspect";
    pub const HOLO_PLAN: &str = "holo.plan";
    pub const HOLO_VERIFY: &str = "holo.verify";
    pub const HOLO_REMOVE: &str = "holo.remove";
    pub const HOLO_LOAD: &str = "holo.load";
    pub const HOLO_UNLOAD: &str = "holo.unload";
    pub const HOLO_RUN: &str = "holo.run";
    pub const HOLO_RESIDENT: &str = "holo.resident";
    pub const HISTORY_CREATE: &str = "history.create";
    pub const HISTORY_LIST: &str = "history.list";
    pub const HISTORY_GET: &str = "history.get";
    pub const HISTORY_APPEND: &str = "history.append";
    pub const HISTORY_DELETE: &str = "history.delete";
    pub const HISTORY_ARCHIVE: &str = "history.archive";
    pub const CHAT_SEND: &str = "chat.send";
    pub const MODEL_LIST: &str = "model.list";
    pub const MODEL_IMPORT: &str = "model.import";
    pub const MODEL_REMOVE: &str = "model.remove";
    pub const NODES_LIST: &str = "nodes.list";
    pub const NODES_HEARTBEAT: &str = "nodes.heartbeat";
    pub const PLUGIN_LIST: &str = "plugin.list";
    pub const PLUGIN_CALL: &str = "plugin.call";
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    Read,
    Mutation,
    Stream,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct OperationInfo {
    pub id: String,
    pub kind: OperationKind,
    pub fallback_safe_before_dispatch: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ModuleInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    pub state: String,
    pub dependencies: Vec<String>,
    pub operations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CapabilityManifest {
    pub protocol_version: u16,
    pub server_version: String,
    pub server_id: String,
    pub role: String,
    pub operations: Vec<OperationInfo>,
    pub modules: Vec<ModuleInfo>,
    pub maximum_message_bytes: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
    pub role: String,
    pub modules_ready: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ObjectMetadata {
    pub id: String,
    pub kind: String,
    pub media_type: String,
    pub filename: Option<String>,
    pub size: u64,
    pub created_at_millis: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectContent {
    pub metadata: ObjectMetadata,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct HoloSection {
    pub kind: String,
    pub offset: u64,
    pub length: u64,
}

/// One ordered application layer in the queryable `.holo` directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct HoloLayer {
    pub position: u32,
    pub kind: String,
    pub content_kappa: String,
    pub entry: String,
    pub architecture: Option<String>,
    pub surface: Option<String>,
    pub engine: Option<String>,
}

/// One composed child application and its attenuated capability set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct HoloChild {
    pub position: u32,
    pub application_kappa: String,
    pub capabilities_kappa: String,
}

/// One content-addressed blob physically embedded in a fat `.holo`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct HoloBlob {
    pub kappa: String,
    pub byte_length: u64,
}

/// A normalized, versioned projection of an application manifest and its
/// physical packaging. Layers refer to blobs by kappa instead of nesting them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct HoloDirectory {
    pub schema_version: u16,
    pub primary_layer: Option<u32>,
    pub requires_kappa: String,
    pub layers: Vec<HoloLayer>,
    pub children: Vec<HoloChild>,
    pub blobs: Vec<HoloBlob>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct HoloInspection {
    /// Physical archive object kappa (BLAKE3 of the complete file).
    pub kappa: String,
    /// Canonical application-manifest kappa, absent for structural archives
    /// that do not contain an application manifest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub application_kappa: Option<String>,
    pub name: String,
    pub format_version: u16,
    pub byte_length: u64,
    pub archive_fingerprint: String,
    pub footer_verified: bool,
    pub sections: Vec<HoloSection>,
    pub directory: Option<HoloDirectory>,
    pub directory_embedded: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct HoloPlanObject {
    pub kappa: String,
    pub resolution_source: Option<String>,
    pub byte_length: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct HoloPlanProvider {
    pub status: String,
    pub name: Option<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct HoloPlanLayer {
    pub position: u32,
    pub kind: String,
    pub content_kappa: String,
    pub entry: String,
    pub architecture: Option<String>,
    pub surface: Option<String>,
    pub engine: Option<String>,
    pub primary: bool,
    pub resolution_source: Option<String>,
    pub byte_length: Option<u64>,
    pub provider: HoloPlanProvider,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct HoloPlanLimits {
    pub max_layers: u64,
    pub max_applications: u64,
    pub max_depth: u64,
    pub max_objects: u64,
    pub max_resolved_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct HoloPlanBlocker {
    pub kind: String,
    pub error_code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct HoloPlan {
    pub archive_kappa: String,
    pub archive_fingerprint: String,
    pub application_kappa: String,
    pub execution_target: String,
    pub packaging: String,
    pub primary_layer: Option<u32>,
    pub capabilities: HoloPlanObject,
    pub layers: Vec<HoloPlanLayer>,
    pub children: Vec<HoloChild>,
    pub resolved_object_count: u64,
    pub resolved_bytes: u64,
    pub application_count: u64,
    pub max_depth: u64,
    pub limits: HoloPlanLimits,
    pub runnable: bool,
    pub blockers: Vec<HoloPlanBlocker>,
}

impl HoloPlan {
    pub fn from_report(report: &ApplicationPlanReport, execution_target: &str) -> Self {
        let layers = report
            .layers
            .iter()
            .map(|layer| {
                let object = report.objects.get(&layer.content_kappa);
                let (architecture, surface, engine) = match layer.kind {
                    LayerKind::RootfsImage => (Some(layer.aux.clone()), None, None),
                    LayerKind::View => (None, Some(layer.aux.clone()), None),
                    LayerKind::InferenceModel => (None, None, Some(layer.aux.clone())),
                    LayerKind::WasmCodemodule | LayerKind::TensorPlan => (None, None, None),
                };
                HoloPlanLayer {
                    position: layer.position,
                    kind: layer_kind_name(layer.kind).to_owned(),
                    content_kappa: layer.content_kappa.clone(),
                    entry: layer.entry.clone(),
                    architecture,
                    surface,
                    engine,
                    primary: layer.primary,
                    resolution_source: layer.resolution_source.as_ref().map(resolution_source_name),
                    byte_length: object
                        .map(|object| object.bytes.len().try_into().unwrap_or(u64::MAX)),
                    provider: provider_report(&layer.provider),
                }
            })
            .collect();
        let children = report
            .children
            .iter()
            .map(|child| HoloChild {
                position: child.position,
                application_kappa: child.application_kappa.clone(),
                capabilities_kappa: child.capabilities_kappa.clone(),
            })
            .collect();
        let blockers = report
            .blockers
            .iter()
            .map(|blocker| HoloPlanBlocker {
                kind: blocker.kind().to_owned(),
                error_code: blocker.error_code().to_owned(),
                message: blocker.message(&report.identity.application_kappa),
            })
            .collect();
        Self {
            archive_kappa: report.identity.archive_kappa.clone(),
            archive_fingerprint: report.identity.archive_fingerprint.clone(),
            application_kappa: report.identity.application_kappa.clone(),
            execution_target: execution_target.to_owned(),
            packaging: packaging(report),
            primary_layer: report.primary_layer,
            capabilities: plan_object(report, &report.requires_kappa),
            layers,
            children,
            resolved_object_count: report.objects.len().try_into().unwrap_or(u64::MAX),
            resolved_bytes: report.resolved_bytes,
            application_count: report.application_count.try_into().unwrap_or(u64::MAX),
            max_depth: report.max_depth.try_into().unwrap_or(u64::MAX),
            limits: HoloPlanLimits {
                max_layers: report.limits.max_layers.try_into().unwrap_or(u64::MAX),
                max_applications: report
                    .limits
                    .max_applications
                    .try_into()
                    .unwrap_or(u64::MAX),
                max_depth: report.limits.max_depth.try_into().unwrap_or(u64::MAX),
                max_objects: report.limits.max_objects.try_into().unwrap_or(u64::MAX),
                max_resolved_bytes: report.limits.max_resolved_bytes,
            },
            runnable: report.runnable(),
            blockers,
        }
    }
}

fn plan_object(report: &ApplicationPlanReport, kappa: &str) -> HoloPlanObject {
    let object = report.objects.get(kappa);
    HoloPlanObject {
        kappa: kappa.to_owned(),
        resolution_source: object.map(|object| resolution_source_name(&object.source)),
        byte_length: object.map(|object| object.bytes.len().try_into().unwrap_or(u64::MAX)),
    }
}

fn provider_report(provider: &ProviderAvailability) -> HoloPlanProvider {
    match provider {
        ProviderAvailability::Unchecked => HoloPlanProvider {
            status: "unchecked".to_owned(),
            name: None,
            reason: None,
        },
        ProviderAvailability::Available { provider } => HoloPlanProvider {
            status: "available".to_owned(),
            name: Some(provider.clone()),
            reason: None,
        },
        ProviderAvailability::Unavailable { reason } => HoloPlanProvider {
            status: "unavailable".to_owned(),
            name: None,
            reason: Some(reason.clone()),
        },
    }
}

fn packaging(report: &ApplicationPlanReport) -> String {
    if report.embedded_object_count == 0 {
        "thin"
    } else if report.embedded_object_count >= report.referenced_object_count {
        "fat"
    } else {
        "hybrid"
    }
    .to_owned()
}

fn resolution_source_name(source: &ResolutionSource) -> String {
    match source {
        ResolutionSource::Embedded => "embedded".to_owned(),
        ResolutionSource::LocalStore => "local_store".to_owned(),
        ResolutionSource::ConfiguredResolver(name) => format!("configured:{name}"),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ResidentHolo {
    pub kappa: String,
    pub state: String,
    pub input_count: usize,
    pub output_count: usize,
    pub resident_bytes: usize,
    pub queued: usize,
    pub processed: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct HoloRunResult {
    pub kappa: String,
    pub outputs: Vec<Vec<u8>>,
    pub elapsed_micros: u64,
    pub resident_bytes: usize,
    #[serde(default)]
    pub requested_capabilities_kappa: String,
    #[serde(default)]
    pub effective_grant_kappa: String,
    #[serde(default)]
    pub grant_source: String,
    #[serde(default)]
    pub authorization: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ConversationMessage {
    pub role: String,
    pub content: String,
    pub created_at_millis: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Conversation {
    pub id: String,
    pub title: String,
    pub created_at_millis: u64,
    pub updated_at_millis: u64,
    pub messages: Vec<ConversationMessage>,
    /// Defaulted so conversations written before archiving existed still load.
    #[serde(default)]
    pub archived: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct NodeRecord {
    pub node_id: String,
    pub version: String,
    pub operations: Vec<String>,
    pub last_seen_millis: u64,
}

/// Runtime status of one allowlisted subprocess plugin.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PluginStatus {
    pub id: String,
    pub name: String,
    pub version: String,
    pub operations: Vec<String>,
    pub running: bool,
    pub restart_count: u64,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RpcRequest {
    Handshake,
    Health,
    Shutdown,
    ModulesList,
    TracingGet,
    TracingSet {
        filter: String,
    },
    RegistryList,
    RegistryPut {
        kind: String,
        media_type: String,
        filename: Option<String>,
        bytes: Vec<u8>,
    },
    RegistryGet {
        id: String,
    },
    FilesList,
    FilesPut {
        media_type: String,
        filename: Option<String>,
        bytes: Vec<u8>,
    },
    FilesGet {
        id: String,
    },
    FilesRename {
        id: String,
        filename: String,
    },
    HoloImport {
        name: String,
        bytes: Vec<u8>,
    },
    HoloList,
    HoloInspect {
        kappa: String,
    },
    HoloPlan {
        kappa: String,
    },
    HoloVerify {
        kappa: String,
    },
    HoloRemove {
        kappa: String,
    },
    HoloLoad {
        kappa: String,
    },
    HoloUnload {
        kappa: String,
    },
    HoloRun {
        kappa: String,
        inputs: Vec<Vec<u8>>,
    },
    HoloResident,
    HistoryCreate {
        title: String,
    },
    HistoryList {
        #[serde(default)]
        include_archived: bool,
    },
    HistoryGet {
        id: String,
    },
    HistoryAppend {
        id: String,
        role: String,
        content: String,
    },
    HistoryDelete {
        id: String,
    },
    HistoryArchive {
        id: String,
        archived: bool,
    },
    ChatSend {
        id: String,
        content: String,
    },
    ModelList,
    ModelImport {
        /// Local path of a `.wcpu` artifact directory readable by the daemon.
        path: String,
    },
    ModelRemove {
        id: String,
    },
    NodesList,
    NodeHeartbeat {
        node: NodeRecord,
    },
    PluginList,
    PluginCall {
        plugin_id: String,
        operation: String,
        /// JSON payload forwarded verbatim to the plugin.
        payload: String,
    },
}

impl RpcRequest {
    pub const fn operation(&self) -> &'static str {
        match self {
            Self::Handshake => operation::SYSTEM_HANDSHAKE,
            Self::Health => operation::SYSTEM_HEALTH,
            Self::Shutdown => operation::SYSTEM_SHUTDOWN,
            Self::ModulesList => operation::MODULES_LIST,
            Self::TracingGet => operation::TRACING_GET,
            Self::TracingSet { .. } => operation::TRACING_SET,
            Self::RegistryList => operation::REGISTRY_LIST,
            Self::RegistryPut { .. } => operation::REGISTRY_PUT,
            Self::RegistryGet { .. } => operation::REGISTRY_GET,
            Self::FilesList => operation::FILES_LIST,
            Self::FilesPut { .. } => operation::FILES_PUT,
            Self::FilesGet { .. } => operation::FILES_GET,
            Self::FilesRename { .. } => operation::FILES_RENAME,
            Self::HoloImport { .. } => operation::HOLO_IMPORT,
            Self::HoloList => operation::HOLO_LIST,
            Self::HoloInspect { .. } => operation::HOLO_INSPECT,
            Self::HoloPlan { .. } => operation::HOLO_PLAN,
            Self::HoloVerify { .. } => operation::HOLO_VERIFY,
            Self::HoloRemove { .. } => operation::HOLO_REMOVE,
            Self::HoloLoad { .. } => operation::HOLO_LOAD,
            Self::HoloUnload { .. } => operation::HOLO_UNLOAD,
            Self::HoloRun { .. } => operation::HOLO_RUN,
            Self::HoloResident => operation::HOLO_RESIDENT,
            Self::HistoryCreate { .. } => operation::HISTORY_CREATE,
            Self::HistoryList { .. } => operation::HISTORY_LIST,
            Self::HistoryGet { .. } => operation::HISTORY_GET,
            Self::HistoryAppend { .. } => operation::HISTORY_APPEND,
            Self::HistoryDelete { .. } => operation::HISTORY_DELETE,
            Self::HistoryArchive { .. } => operation::HISTORY_ARCHIVE,
            Self::ChatSend { .. } => operation::CHAT_SEND,
            Self::ModelList => operation::MODEL_LIST,
            Self::ModelImport { .. } => operation::MODEL_IMPORT,
            Self::ModelRemove { .. } => operation::MODEL_REMOVE,
            Self::NodesList => operation::NODES_LIST,
            Self::NodeHeartbeat { .. } => operation::NODES_HEARTBEAT,
            Self::PluginList => operation::PLUGIN_LIST,
            Self::PluginCall { .. } => operation::PLUGIN_CALL,
        }
    }

    pub const fn kind(&self) -> OperationKind {
        match self {
            Self::Handshake
            | Self::Health
            | Self::ModulesList
            | Self::TracingGet
            | Self::RegistryList
            | Self::RegistryGet { .. }
            | Self::FilesList
            | Self::FilesGet { .. }
            | Self::HoloList
            | Self::HoloInspect { .. }
            | Self::HoloPlan { .. }
            | Self::HoloVerify { .. }
            | Self::HoloResident
            | Self::HistoryList { .. }
            | Self::HistoryGet { .. }
            | Self::ModelList
            | Self::NodesList
            | Self::PluginList => OperationKind::Read,
            Self::HoloRun { .. } => OperationKind::Stream,
            _ => OperationKind::Mutation,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RpcResponse {
    CapabilityManifest(CapabilityManifest),
    Health(HealthResponse),
    Modules(Vec<ModuleInfo>),
    Objects(Vec<ObjectMetadata>),
    Object(ObjectMetadata),
    ObjectContent(ObjectContent),
    HoloInspection(HoloInspection),
    HoloPlan(HoloPlan),
    HoloList(Vec<HoloInspection>),
    HoloResident(Vec<ResidentHolo>),
    HoloRun(HoloRunResult),
    Conversation(Conversation),
    Conversations(Vec<Conversation>),
    Model(crate::models::ModelInfo),
    Models(Vec<crate::models::ModelInfo>),
    Nodes(Vec<NodeRecord>),
    /// Raw JSON result string returned by a plugin invocation.
    PluginResult(String),
    Plugins(Vec<PluginStatus>),
    TracingFilter(String),
    Accepted,
    Error(ApiError),
}

impl RpcResponse {
    pub fn from_result<T>(result: Result<T, LiveError>, map: impl FnOnce(T) -> Self) -> Self {
        match result {
            Ok(value) => map(value),
            Err(error) => Self::Error(ApiError::from(&error)),
        }
    }
}
