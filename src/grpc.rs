use crate::app::AppState;
use crate::auth::Principal;
use crate::error::{ApiError, LiveError, Result};
use crate::protocol::{
    CapabilityManifest, Conversation, ConversationMessage, HealthResponse, HoloInspection,
    HoloRunResult, HoloSection, ModuleInfo, NodeRecord, ObjectContent, ObjectMetadata,
    OperationInfo, OperationKind, ResidentHolo, RpcRequest, RpcResponse,
};
use crate::util::constant_time_eq;
use opentelemetry::metrics::{Counter, Histogram};
use opentelemetry::KeyValue;
use std::time::Instant;
use tonic::{Request, Response, Status};
use tracing::Instrument;

#[allow(clippy::all, clippy::pedantic)]
pub mod pb {
    tonic::include_proto!("hologram.live.v1");
}

#[derive(Clone)]
struct GrpcTransport {
    state: AppState,
    requests: Counter<u64>,
    request_duration_ms: Histogram<f64>,
}

#[tonic::async_trait]
impl pb::hologram_live_server::HologramLive for GrpcTransport {
    async fn call(
        &self,
        request: Request<pb::RpcRequest>,
    ) -> std::result::Result<Response<pb::RpcResponse>, Status> {
        let principal = principal_from_metadata(&self.state, request.metadata())?;
        let request = RpcRequest::try_from(request.into_inner()).map_err(live_error_to_status)?;
        let operation = request.operation();
        let started = Instant::now();
        let span = tracing::info_span!(
            "live.grpc.call",
            rpc.system = "grpc",
            rpc.service = "hologram.live.v1.HologramLive",
            rpc.method = "Call",
            operation,
            principal = %principal.id
        );
        let response = async { self.state.dispatch(&principal, request).await }
            .instrument(span)
            .await;
        let outcome = if matches!(response, RpcResponse::Error(_)) {
            "error"
        } else {
            "ok"
        };
        let attributes = [
            KeyValue::new("rpc.method", operation),
            KeyValue::new("rpc.outcome", outcome),
        ];
        self.requests.add(1, &attributes);
        self.request_duration_ms
            .record(started.elapsed().as_secs_f64() * 1_000.0, &attributes);
        Ok(Response::new(response.into()))
    }
}

pub fn router(state: AppState) -> axum::Router {
    let maximum = state.config().server.max_rpc_bytes;
    let meter = opentelemetry::global::meter("hologram-live");
    let service = pb::hologram_live_server::HologramLiveServer::new(GrpcTransport {
        state,
        requests: meter.u64_counter("hologram.rpc.requests").build(),
        request_duration_ms: meter
            .f64_histogram("hologram.rpc.request.duration")
            .with_unit("ms")
            .build(),
    })
    .max_decoding_message_size(maximum)
    .max_encoding_message_size(maximum);
    tonic::service::Routes::new(service).into_axum_router()
}

fn principal_from_metadata(
    state: &AppState,
    metadata: &tonic::metadata::MetadataMap,
) -> std::result::Result<Principal, Status> {
    if !state.config().auth.required {
        return Ok(Principal {
            id: "local-user".to_owned(),
            scope: "local".to_owned(),
        });
    }
    let configured = state
        .config()
        .auth_token()
        .ok_or_else(|| Status::unauthenticated("server authentication token is unavailable"))?;
    let supplied = metadata
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .ok_or_else(|| Status::unauthenticated("missing bearer token"))?;
    if !constant_time_eq(configured.as_bytes(), supplied.as_bytes()) {
        return Err(Status::unauthenticated("invalid bearer token"));
    }
    Ok(Principal {
        id: "token-principal".to_owned(),
        scope: "default".to_owned(),
    })
}

fn live_error_to_status(error: LiveError) -> Status {
    match error {
        LiveError::Authentication(message) => Status::unauthenticated(message),
        LiveError::Authorization(message) => Status::permission_denied(message),
        LiveError::Capability(message) => Status::unimplemented(message),
        LiveError::NotFound(message) => Status::not_found(message),
        LiveError::Conflict(message) => Status::failed_precondition(message),
        LiveError::Protocol(message) | LiveError::Config(message) => {
            Status::invalid_argument(message)
        }
        other => Status::internal(other.to_string()),
    }
}

impl From<RpcRequest> for pb::RpcRequest {
    fn from(value: RpcRequest) -> Self {
        use pb::rpc_request::Request as Wire;
        let empty = || pb::Empty {};
        let request = match value {
            RpcRequest::Handshake => Wire::Handshake(empty()),
            RpcRequest::Health => Wire::Health(empty()),
            RpcRequest::Shutdown => Wire::Shutdown(empty()),
            RpcRequest::ModulesList => Wire::ModulesList(empty()),
            RpcRequest::TracingGet => Wire::TracingGet(empty()),
            RpcRequest::TracingSet { filter } => Wire::TracingSet(pb::TracingSetRequest { filter }),
            RpcRequest::RegistryList => Wire::RegistryList(empty()),
            RpcRequest::RegistryPut {
                kind,
                media_type,
                filename,
                bytes,
            } => Wire::RegistryPut(pb::ObjectPutRequest {
                kind,
                media_type,
                filename,
                content: bytes,
            }),
            RpcRequest::RegistryGet { id } => Wire::RegistryGet(pb::IdRequest { id }),
            RpcRequest::FilesList => Wire::FilesList(empty()),
            RpcRequest::FilesPut {
                media_type,
                filename,
                bytes,
            } => Wire::FilesPut(pb::ObjectPutRequest {
                kind: "file".to_owned(),
                media_type,
                filename,
                content: bytes,
            }),
            RpcRequest::FilesGet { id } => Wire::FilesGet(pb::IdRequest { id }),
            RpcRequest::FilesRename { id, filename } => {
                Wire::FilesRename(pb::FileRenameRequest { id, filename })
            }
            RpcRequest::HoloImport { name, bytes } => Wire::HoloImport(pb::HoloImportRequest {
                name,
                content: bytes,
            }),
            RpcRequest::HoloList => Wire::HoloList(empty()),
            RpcRequest::HoloInspect { kappa } => Wire::HoloInspect(pb::IdRequest { id: kappa }),
            RpcRequest::HoloVerify { kappa } => Wire::HoloVerify(pb::IdRequest { id: kappa }),
            RpcRequest::HoloRemove { kappa } => Wire::HoloRemove(pb::IdRequest { id: kappa }),
            RpcRequest::HoloLoad { kappa } => Wire::HoloLoad(pb::IdRequest { id: kappa }),
            RpcRequest::HoloUnload { kappa } => Wire::HoloUnload(pb::IdRequest { id: kappa }),
            RpcRequest::HoloRun { kappa, inputs } => {
                Wire::HoloRun(pb::HoloRunRequest { kappa, inputs })
            }
            RpcRequest::HoloResident => Wire::HoloResident(empty()),
            RpcRequest::HistoryCreate { title } => {
                Wire::HistoryCreate(pb::HistoryCreateRequest { title })
            }
            RpcRequest::HistoryList { include_archived } => {
                Wire::HistoryList(pb::HistoryListRequest { include_archived })
            }
            RpcRequest::HistoryGet { id } => Wire::HistoryGet(pb::IdRequest { id }),
            RpcRequest::HistoryAppend { id, role, content } => {
                Wire::HistoryAppend(pb::HistoryAppendRequest { id, role, content })
            }
            RpcRequest::HistoryDelete { id } => Wire::HistoryDelete(pb::IdRequest { id }),
            RpcRequest::HistoryArchive { id, archived } => {
                Wire::HistoryArchive(pb::HistoryArchiveRequest { id, archived })
            }
            RpcRequest::ChatSend { id, content } => {
                Wire::ChatSend(pb::ChatSendRequest { id, content })
            }
            RpcRequest::NodesList => Wire::NodesList(empty()),
            RpcRequest::NodeHeartbeat { node } => Wire::NodeHeartbeat(node.into()),
        };
        Self {
            request: Some(request),
        }
    }
}

impl TryFrom<pb::RpcRequest> for RpcRequest {
    type Error = LiveError;

    fn try_from(value: pb::RpcRequest) -> Result<Self> {
        use pb::rpc_request::Request as Wire;
        match value
            .request
            .ok_or_else(|| LiveError::Protocol("gRPC request has no operation".to_owned()))?
        {
            Wire::Handshake(_) => Ok(Self::Handshake),
            Wire::Health(_) => Ok(Self::Health),
            Wire::Shutdown(_) => Ok(Self::Shutdown),
            Wire::ModulesList(_) => Ok(Self::ModulesList),
            Wire::TracingGet(_) => Ok(Self::TracingGet),
            Wire::TracingSet(value) => Ok(Self::TracingSet {
                filter: value.filter,
            }),
            Wire::RegistryList(_) => Ok(Self::RegistryList),
            Wire::RegistryPut(value) => Ok(Self::RegistryPut {
                kind: value.kind,
                media_type: value.media_type,
                filename: value.filename,
                bytes: value.content,
            }),
            Wire::RegistryGet(value) => Ok(Self::RegistryGet { id: value.id }),
            Wire::FilesList(_) => Ok(Self::FilesList),
            Wire::FilesPut(value) => Ok(Self::FilesPut {
                media_type: value.media_type,
                filename: value.filename,
                bytes: value.content,
            }),
            Wire::FilesGet(value) => Ok(Self::FilesGet { id: value.id }),
            Wire::FilesRename(value) => Ok(Self::FilesRename {
                id: value.id,
                filename: value.filename,
            }),
            Wire::HoloImport(value) => Ok(Self::HoloImport {
                name: value.name,
                bytes: value.content,
            }),
            Wire::HoloList(_) => Ok(Self::HoloList),
            Wire::HoloInspect(value) => Ok(Self::HoloInspect { kappa: value.id }),
            Wire::HoloVerify(value) => Ok(Self::HoloVerify { kappa: value.id }),
            Wire::HoloRemove(value) => Ok(Self::HoloRemove { kappa: value.id }),
            Wire::HoloLoad(value) => Ok(Self::HoloLoad { kappa: value.id }),
            Wire::HoloUnload(value) => Ok(Self::HoloUnload { kappa: value.id }),
            Wire::HoloRun(value) => Ok(Self::HoloRun {
                kappa: value.kappa,
                inputs: value.inputs,
            }),
            Wire::HoloResident(_) => Ok(Self::HoloResident),
            Wire::HistoryCreate(value) => Ok(Self::HistoryCreate { title: value.title }),
            Wire::HistoryList(value) => Ok(Self::HistoryList {
                include_archived: value.include_archived,
            }),
            Wire::HistoryGet(value) => Ok(Self::HistoryGet { id: value.id }),
            Wire::HistoryAppend(value) => Ok(Self::HistoryAppend {
                id: value.id,
                role: value.role,
                content: value.content,
            }),
            Wire::HistoryDelete(value) => Ok(Self::HistoryDelete { id: value.id }),
            Wire::HistoryArchive(value) => Ok(Self::HistoryArchive {
                id: value.id,
                archived: value.archived,
            }),
            Wire::ChatSend(value) => Ok(Self::ChatSend {
                id: value.id,
                content: value.content,
            }),
            Wire::NodesList(_) => Ok(Self::NodesList),
            Wire::NodeHeartbeat(value) => Ok(Self::NodeHeartbeat { node: value.into() }),
        }
    }
}

impl From<RpcResponse> for pb::RpcResponse {
    fn from(value: RpcResponse) -> Self {
        use pb::rpc_response::Response as Wire;
        let response = match value {
            RpcResponse::CapabilityManifest(value) => Wire::CapabilityManifest(value.into()),
            RpcResponse::Health(value) => Wire::Health(value.into()),
            RpcResponse::Modules(items) => Wire::Modules(pb::ModuleList {
                items: items.into_iter().map(Into::into).collect(),
            }),
            RpcResponse::Objects(items) => Wire::Objects(pb::ObjectList {
                items: items.into_iter().map(Into::into).collect(),
            }),
            RpcResponse::Object(value) => Wire::Object(value.into()),
            RpcResponse::ObjectContent(value) => Wire::ObjectContent(value.into()),
            RpcResponse::HoloInspection(value) => Wire::HoloInspection(value.into()),
            RpcResponse::HoloList(items) => Wire::HoloList(pb::HoloInspectionList {
                items: items.into_iter().map(Into::into).collect(),
            }),
            RpcResponse::HoloResident(items) => Wire::HoloResident(pb::ResidentHoloList {
                items: items.into_iter().map(Into::into).collect(),
            }),
            RpcResponse::HoloRun(value) => Wire::HoloRun(value.into()),
            RpcResponse::Conversation(value) => Wire::Conversation(value.into()),
            RpcResponse::Conversations(items) => Wire::Conversations(pb::ConversationList {
                items: items.into_iter().map(Into::into).collect(),
            }),
            RpcResponse::Nodes(items) => Wire::Nodes(pb::NodeList {
                items: items.into_iter().map(Into::into).collect(),
            }),
            RpcResponse::TracingFilter(filter) => Wire::TracingFilter(pb::TracingFilter { filter }),
            RpcResponse::Accepted => Wire::Accepted(pb::Empty {}),
            RpcResponse::Error(value) => Wire::Error(value.into()),
        };
        Self {
            response: Some(response),
        }
    }
}

impl TryFrom<pb::RpcResponse> for RpcResponse {
    type Error = LiveError;

    fn try_from(value: pb::RpcResponse) -> Result<Self> {
        use pb::rpc_response::Response as Wire;
        match value
            .response
            .ok_or_else(|| LiveError::Protocol("gRPC response has no body".to_owned()))?
        {
            Wire::CapabilityManifest(value) => Ok(Self::CapabilityManifest(value.try_into()?)),
            Wire::Health(value) => Ok(Self::Health(value.try_into()?)),
            Wire::Modules(value) => Ok(Self::Modules(
                value.items.into_iter().map(Into::into).collect(),
            )),
            Wire::Objects(value) => Ok(Self::Objects(
                value.items.into_iter().map(Into::into).collect(),
            )),
            Wire::Object(value) => Ok(Self::Object(value.into())),
            Wire::ObjectContent(value) => Ok(Self::ObjectContent(value.try_into()?)),
            Wire::HoloInspection(value) => Ok(Self::HoloInspection(value.try_into()?)),
            Wire::HoloList(value) => Ok(Self::HoloList(
                value
                    .items
                    .into_iter()
                    .map(TryInto::try_into)
                    .collect::<Result<_>>()?,
            )),
            Wire::HoloResident(value) => Ok(Self::HoloResident(
                value
                    .items
                    .into_iter()
                    .map(TryInto::try_into)
                    .collect::<Result<_>>()?,
            )),
            Wire::HoloRun(value) => Ok(Self::HoloRun(value.try_into()?)),
            Wire::Conversation(value) => Ok(Self::Conversation(value.into())),
            Wire::Conversations(value) => Ok(Self::Conversations(
                value.items.into_iter().map(Into::into).collect(),
            )),
            Wire::Nodes(value) => Ok(Self::Nodes(
                value.items.into_iter().map(Into::into).collect(),
            )),
            Wire::TracingFilter(value) => Ok(Self::TracingFilter(value.filter)),
            Wire::Accepted(_) => Ok(Self::Accepted),
            Wire::Error(value) => Ok(Self::Error(value.into())),
        }
    }
}

impl From<OperationKind> for pb::OperationKind {
    fn from(value: OperationKind) -> Self {
        match value {
            OperationKind::Read => Self::Read,
            OperationKind::Mutation => Self::Mutation,
            OperationKind::Stream => Self::Stream,
        }
    }
}

impl TryFrom<pb::OperationKind> for OperationKind {
    type Error = LiveError;

    fn try_from(value: pb::OperationKind) -> Result<Self> {
        match value {
            pb::OperationKind::Read => Ok(Self::Read),
            pb::OperationKind::Mutation => Ok(Self::Mutation),
            pb::OperationKind::Stream => Ok(Self::Stream),
            pb::OperationKind::Unspecified => Err(LiveError::Protocol(
                "operation kind is unspecified".to_owned(),
            )),
        }
    }
}

impl From<OperationInfo> for pb::OperationInfo {
    fn from(value: OperationInfo) -> Self {
        Self {
            id: value.id,
            kind: pb::OperationKind::from(value.kind) as i32,
            fallback_safe_before_dispatch: value.fallback_safe_before_dispatch,
        }
    }
}

impl TryFrom<pb::OperationInfo> for OperationInfo {
    type Error = LiveError;

    fn try_from(value: pb::OperationInfo) -> Result<Self> {
        let kind = pb::OperationKind::try_from(value.kind)
            .map_err(|_| LiveError::Protocol(format!("unknown operation kind {}", value.kind)))?
            .try_into()?;
        Ok(Self {
            id: value.id,
            kind,
            fallback_safe_before_dispatch: value.fallback_safe_before_dispatch,
        })
    }
}

impl From<ModuleInfo> for pb::ModuleInfo {
    fn from(value: ModuleInfo) -> Self {
        Self {
            id: value.id,
            name: value.name,
            version: value.version,
            state: value.state,
            dependencies: value.dependencies,
            operations: value.operations,
        }
    }
}

impl From<pb::ModuleInfo> for ModuleInfo {
    fn from(value: pb::ModuleInfo) -> Self {
        Self {
            id: value.id,
            name: value.name,
            version: value.version,
            state: value.state,
            dependencies: value.dependencies,
            operations: value.operations,
        }
    }
}

impl From<CapabilityManifest> for pb::CapabilityManifest {
    fn from(value: CapabilityManifest) -> Self {
        Self {
            protocol_version: u32::from(value.protocol_version),
            server_version: value.server_version,
            server_id: value.server_id,
            role: value.role,
            operations: value.operations.into_iter().map(Into::into).collect(),
            modules: value.modules.into_iter().map(Into::into).collect(),
            maximum_message_bytes: value.maximum_message_bytes,
        }
    }
}

impl TryFrom<pb::CapabilityManifest> for CapabilityManifest {
    type Error = LiveError;

    fn try_from(value: pb::CapabilityManifest) -> Result<Self> {
        Ok(Self {
            protocol_version: narrow(value.protocol_version, "protocol_version")?,
            server_version: value.server_version,
            server_id: value.server_id,
            role: value.role,
            operations: value
                .operations
                .into_iter()
                .map(TryInto::try_into)
                .collect::<Result<_>>()?,
            modules: value.modules.into_iter().map(Into::into).collect(),
            maximum_message_bytes: value.maximum_message_bytes,
        })
    }
}

impl From<HealthResponse> for pb::HealthResponse {
    fn from(value: HealthResponse) -> Self {
        Self {
            status: value.status,
            version: value.version,
            role: value.role,
            modules_ready: value.modules_ready.try_into().unwrap_or(u64::MAX),
        }
    }
}

impl TryFrom<pb::HealthResponse> for HealthResponse {
    type Error = LiveError;

    fn try_from(value: pb::HealthResponse) -> Result<Self> {
        Ok(Self {
            status: value.status,
            version: value.version,
            role: value.role,
            modules_ready: narrow(value.modules_ready, "modules_ready")?,
        })
    }
}

impl From<ObjectMetadata> for pb::ObjectMetadata {
    fn from(value: ObjectMetadata) -> Self {
        Self {
            id: value.id,
            kind: value.kind,
            media_type: value.media_type,
            filename: value.filename,
            size: value.size,
            created_at_millis: value.created_at_millis,
        }
    }
}

impl From<pb::ObjectMetadata> for ObjectMetadata {
    fn from(value: pb::ObjectMetadata) -> Self {
        Self {
            id: value.id,
            kind: value.kind,
            media_type: value.media_type,
            filename: value.filename,
            size: value.size,
            created_at_millis: value.created_at_millis,
        }
    }
}

impl From<ObjectContent> for pb::ObjectContent {
    fn from(value: ObjectContent) -> Self {
        Self {
            metadata: Some(value.metadata.into()),
            content: value.bytes,
        }
    }
}

impl TryFrom<pb::ObjectContent> for ObjectContent {
    type Error = LiveError;

    fn try_from(value: pb::ObjectContent) -> Result<Self> {
        Ok(Self {
            metadata: value
                .metadata
                .ok_or_else(|| {
                    LiveError::Protocol("object content response has no metadata".to_owned())
                })?
                .into(),
            bytes: value.content,
        })
    }
}

impl From<HoloSection> for pb::HoloSection {
    fn from(value: HoloSection) -> Self {
        Self {
            kind: value.kind,
            offset: value.offset,
            length: value.length,
        }
    }
}

impl From<pb::HoloSection> for HoloSection {
    fn from(value: pb::HoloSection) -> Self {
        Self {
            kind: value.kind,
            offset: value.offset,
            length: value.length,
        }
    }
}

impl From<HoloInspection> for pb::HoloInspection {
    fn from(value: HoloInspection) -> Self {
        Self {
            kappa: value.kappa,
            name: value.name,
            format_version: u32::from(value.format_version),
            byte_length: value.byte_length,
            archive_fingerprint: value.archive_fingerprint,
            footer_verified: value.footer_verified,
            sections: value.sections.into_iter().map(Into::into).collect(),
        }
    }
}

impl TryFrom<pb::HoloInspection> for HoloInspection {
    type Error = LiveError;

    fn try_from(value: pb::HoloInspection) -> Result<Self> {
        Ok(Self {
            kappa: value.kappa,
            name: value.name,
            format_version: narrow(value.format_version, "format_version")?,
            byte_length: value.byte_length,
            archive_fingerprint: value.archive_fingerprint,
            footer_verified: value.footer_verified,
            sections: value.sections.into_iter().map(Into::into).collect(),
        })
    }
}

impl From<ResidentHolo> for pb::ResidentHolo {
    fn from(value: ResidentHolo) -> Self {
        Self {
            kappa: value.kappa,
            input_count: value.input_count.try_into().unwrap_or(u64::MAX),
            output_count: value.output_count.try_into().unwrap_or(u64::MAX),
            resident_bytes: value.resident_bytes.try_into().unwrap_or(u64::MAX),
            queued: value.queued.try_into().unwrap_or(u64::MAX),
            processed: value.processed.try_into().unwrap_or(u64::MAX),
        }
    }
}

impl TryFrom<pb::ResidentHolo> for ResidentHolo {
    type Error = LiveError;

    fn try_from(value: pb::ResidentHolo) -> Result<Self> {
        Ok(Self {
            kappa: value.kappa,
            input_count: narrow(value.input_count, "input_count")?,
            output_count: narrow(value.output_count, "output_count")?,
            resident_bytes: narrow(value.resident_bytes, "resident_bytes")?,
            queued: narrow(value.queued, "queued")?,
            processed: narrow(value.processed, "processed")?,
        })
    }
}

impl From<HoloRunResult> for pb::HoloRunResult {
    fn from(value: HoloRunResult) -> Self {
        Self {
            kappa: value.kappa,
            outputs: value.outputs,
            elapsed_micros: value.elapsed_micros,
            resident_bytes: value.resident_bytes.try_into().unwrap_or(u64::MAX),
        }
    }
}

impl TryFrom<pb::HoloRunResult> for HoloRunResult {
    type Error = LiveError;

    fn try_from(value: pb::HoloRunResult) -> Result<Self> {
        Ok(Self {
            kappa: value.kappa,
            outputs: value.outputs,
            elapsed_micros: value.elapsed_micros,
            resident_bytes: narrow(value.resident_bytes, "resident_bytes")?,
        })
    }
}

impl From<ConversationMessage> for pb::ConversationMessage {
    fn from(value: ConversationMessage) -> Self {
        Self {
            role: value.role,
            content: value.content,
            created_at_millis: value.created_at_millis,
        }
    }
}

impl From<pb::ConversationMessage> for ConversationMessage {
    fn from(value: pb::ConversationMessage) -> Self {
        Self {
            role: value.role,
            content: value.content,
            created_at_millis: value.created_at_millis,
        }
    }
}

impl From<Conversation> for pb::Conversation {
    fn from(value: Conversation) -> Self {
        Self {
            id: value.id,
            title: value.title,
            created_at_millis: value.created_at_millis,
            updated_at_millis: value.updated_at_millis,
            messages: value.messages.into_iter().map(Into::into).collect(),
            archived: value.archived,
        }
    }
}

impl From<pb::Conversation> for Conversation {
    fn from(value: pb::Conversation) -> Self {
        Self {
            id: value.id,
            title: value.title,
            created_at_millis: value.created_at_millis,
            updated_at_millis: value.updated_at_millis,
            messages: value.messages.into_iter().map(Into::into).collect(),
            archived: value.archived,
        }
    }
}

impl From<NodeRecord> for pb::NodeRecord {
    fn from(value: NodeRecord) -> Self {
        Self {
            node_id: value.node_id,
            version: value.version,
            operations: value.operations,
            last_seen_millis: value.last_seen_millis,
        }
    }
}

impl From<pb::NodeRecord> for NodeRecord {
    fn from(value: pb::NodeRecord) -> Self {
        Self {
            node_id: value.node_id,
            version: value.version,
            operations: value.operations,
            last_seen_millis: value.last_seen_millis,
        }
    }
}

impl From<ApiError> for pb::ApiError {
    fn from(value: ApiError) -> Self {
        Self {
            code: value.code,
            message: value.message,
        }
    }
}

impl From<pb::ApiError> for ApiError {
    fn from(value: pb::ApiError) -> Self {
        Self {
            code: value.code,
            message: value.message,
        }
    }
}

fn narrow<T, U>(value: T, field: &str) -> Result<U>
where
    U: TryFrom<T>,
{
    value
        .try_into()
        .map_err(|_| LiveError::Protocol(format!("{field} does not fit this platform")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_round_trip_preserves_payload() {
        let request = RpcRequest::HistoryAppend {
            id: "thread".to_owned(),
            role: "user".to_owned(),
            content: "hello".to_owned(),
        };
        let decoded = RpcRequest::try_from(pb::RpcRequest::from(request.clone())).expect("decode");
        assert_eq!(decoded.operation(), request.operation());
        assert!(matches!(
            decoded,
            RpcRequest::HistoryAppend { content, .. } if content == "hello"
        ));

        let request = RpcRequest::ChatSend {
            id: "blake3:chat".to_owned(),
            content: "echo me".to_owned(),
        };
        let decoded = RpcRequest::try_from(pb::RpcRequest::from(request)).expect("decode chat");
        assert!(matches!(
            decoded,
            RpcRequest::ChatSend { id, content }
                if id == "blake3:chat" && content == "echo me"
        ));

        let request = RpcRequest::FilesRename {
            id: "blake3:file".to_owned(),
            filename: "notes.txt".to_owned(),
        };
        let decoded = RpcRequest::try_from(pb::RpcRequest::from(request)).expect("decode rename");
        assert!(matches!(
            decoded,
            RpcRequest::FilesRename { id, filename }
                if id == "blake3:file" && filename == "notes.txt"
        ));
    }

    #[test]
    fn object_round_trips_preserve_bytes_and_metadata() {
        let request = RpcRequest::FilesPut {
            media_type: "text/plain".to_owned(),
            filename: Some("hello.txt".to_owned()),
            bytes: b"hello".to_vec(),
        };
        let decoded = RpcRequest::try_from(pb::RpcRequest::from(request)).expect("decode request");
        assert!(matches!(
            decoded,
            RpcRequest::FilesPut { media_type, filename: Some(filename), bytes }
                if media_type == "text/plain" && filename == "hello.txt" && bytes == b"hello"
        ));

        let response = RpcResponse::ObjectContent(ObjectContent {
            metadata: ObjectMetadata {
                id: "blake3:abc".to_owned(),
                kind: "file".to_owned(),
                media_type: "text/plain".to_owned(),
                filename: Some("hello.txt".to_owned()),
                size: 5,
                created_at_millis: 1,
            },
            bytes: b"hello".to_vec(),
        });
        let decoded =
            RpcResponse::try_from(pb::RpcResponse::from(response)).expect("decode response");
        assert!(matches!(
            decoded,
            RpcResponse::ObjectContent(ObjectContent { metadata, bytes })
                if metadata.filename.as_deref() == Some("hello.txt") && bytes == b"hello"
        ));
    }
}
