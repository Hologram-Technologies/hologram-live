use crate::error::{ApiError, LiveError};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

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
    pub const HOLO_IMPORT: &str = "holo.import";
    pub const HOLO_LIST: &str = "holo.list";
    pub const HOLO_INSPECT: &str = "holo.inspect";
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
    pub const NODES_LIST: &str = "nodes.list";
    pub const NODES_HEARTBEAT: &str = "nodes.heartbeat";
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

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct HoloInspection {
    pub kappa: String,
    pub name: String,
    pub format_version: u16,
    pub byte_length: u64,
    pub archive_fingerprint: String,
    pub footer_verified: bool,
    pub sections: Vec<HoloSection>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ResidentHolo {
    pub kappa: String,
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
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct NodeRecord {
    pub node_id: String,
    pub version: String,
    pub operations: Vec<String>,
    pub last_seen_millis: u64,
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
    HoloImport {
        name: String,
        bytes: Vec<u8>,
    },
    HoloList,
    HoloInspect {
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
    HistoryList,
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
    NodesList,
    NodeHeartbeat {
        node: NodeRecord,
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
            Self::HoloImport { .. } => operation::HOLO_IMPORT,
            Self::HoloList => operation::HOLO_LIST,
            Self::HoloInspect { .. } => operation::HOLO_INSPECT,
            Self::HoloVerify { .. } => operation::HOLO_VERIFY,
            Self::HoloRemove { .. } => operation::HOLO_REMOVE,
            Self::HoloLoad { .. } => operation::HOLO_LOAD,
            Self::HoloUnload { .. } => operation::HOLO_UNLOAD,
            Self::HoloRun { .. } => operation::HOLO_RUN,
            Self::HoloResident => operation::HOLO_RESIDENT,
            Self::HistoryCreate { .. } => operation::HISTORY_CREATE,
            Self::HistoryList => operation::HISTORY_LIST,
            Self::HistoryGet { .. } => operation::HISTORY_GET,
            Self::HistoryAppend { .. } => operation::HISTORY_APPEND,
            Self::HistoryDelete { .. } => operation::HISTORY_DELETE,
            Self::NodesList => operation::NODES_LIST,
            Self::NodeHeartbeat { .. } => operation::NODES_HEARTBEAT,
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
            | Self::HoloVerify { .. }
            | Self::HoloResident
            | Self::HistoryList
            | Self::HistoryGet { .. }
            | Self::NodesList => OperationKind::Read,
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
    HoloList(Vec<HoloInspection>),
    HoloResident(Vec<ResidentHolo>),
    HoloRun(HoloRunResult),
    Conversation(Conversation),
    Conversations(Vec<Conversation>),
    Nodes(Vec<NodeRecord>),
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
