use crate::app::AppState;
use crate::module::{LiveModule, ModuleDescriptor, OperationDescriptor};
use crate::modules::{registry, HttpError};
use crate::protocol::{operation, ObjectMetadata, OperationKind};
use axum::body::{Body, Bytes};
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::Response;
use axum::routing::get;
use axum::{Json, Router};

const OPERATIONS: &[OperationDescriptor] = &[
    OperationDescriptor {
        id: operation::FILES_LIST,
        kind: OperationKind::Read,
        fallback_safe_before_dispatch: true,
    },
    OperationDescriptor {
        id: operation::FILES_GET,
        kind: OperationKind::Read,
        fallback_safe_before_dispatch: true,
    },
    OperationDescriptor {
        id: operation::FILES_PUT,
        kind: OperationKind::Mutation,
        fallback_safe_before_dispatch: false,
    },
    OperationDescriptor {
        id: operation::FILES_RENAME,
        kind: OperationKind::Mutation,
        fallback_safe_before_dispatch: false,
    },
];

static DESCRIPTOR: ModuleDescriptor = ModuleDescriptor {
    id: "dev.hologram.live.files",
    name: "Artifact Files",
    version: env!("CARGO_PKG_VERSION"),
    dependencies: &["dev.hologram.live.kappa-registry"],
    operations: OPERATIONS,
};

pub struct FilesModule;

impl LiveModule for FilesModule {
    fn descriptor(&self) -> &'static ModuleDescriptor {
        &DESCRIPTOR
    }

    fn router(&self) -> Router<AppState> {
        Router::new()
            .route("/api/v1/files", get(list_files).post(put_file))
            .route("/api/v1/files/{id}", get(get_file).patch(rename_file))
    }

    fn openapi(&self) -> utoipa::openapi::OpenApi {
        <FilesApiDoc as utoipa::OpenApi>::openapi()
    }
}

#[derive(utoipa::OpenApi)]
#[openapi(
    paths(list_files, put_file, get_file, rename_file),
    components(schemas(ObjectMetadata, RenameFileRequest)),
    tags((name = "files", description = "Artifact file discovery"))
)]
struct FilesApiDoc;

#[derive(serde::Deserialize, utoipa::ToSchema)]
pub struct RenameFileRequest {
    pub filename: String,
}

#[utoipa::path(
    get,
    path = "/api/v1/files",
    responses((status = 200, body = [ObjectMetadata]))
)]
pub async fn list_files(
    State(state): State<AppState>,
) -> Result<Json<Vec<ObjectMetadata>>, HttpError> {
    let provider = state.registry().clone();
    let files = tokio::task::spawn_blocking(move || provider.list_objects(Some("file")))
        .await
        .map_err(|error| {
            crate::error::LiveError::Conflict(format!("join file listing: {error}"))
        })??;
    Ok(Json(files))
}

#[utoipa::path(
    post,
    path = "/api/v1/files",
    request_body(content = Vec<u8>, content_type = "application/octet-stream"),
    params(
        ("content-type" = Option<String>, Header, description = "Stored media type"),
        ("x-hologram-filename" = Option<String>, Header, description = "Original filename")
    ),
    responses((status = 201, body = ObjectMetadata))
)]
pub async fn put_file(
    State(state): State<AppState>,
    headers: HeaderMap,
    bytes: Bytes,
) -> Result<(StatusCode, Json<ObjectMetadata>), HttpError> {
    let media_type = registry::optional_header(&headers, header::CONTENT_TYPE.as_str())?
        .unwrap_or_else(|| "application/octet-stream".to_owned());
    let filename = registry::optional_header(&headers, "x-hologram-filename")?;
    let metadata = registry::store_object(
        &state,
        "file".to_owned(),
        media_type,
        filename,
        bytes.to_vec(),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(metadata)))
}

#[utoipa::path(
    get,
    path = "/api/v1/files/{id}",
    params(("id" = String, Path, description = "Content-addressed file ID")),
    responses(
        (status = 200, description = "Raw file bytes", content_type = "application/octet-stream"),
        (status = 404, description = "File not found")
    )
)]
pub async fn get_file(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response<Body>, HttpError> {
    registry::get_object(State(state), Path(id)).await
}

#[utoipa::path(
    patch,
    path = "/api/v1/files/{id}",
    params(("id" = String, Path, description = "Content-addressed file ID")),
    request_body = RenameFileRequest,
    responses(
        (status = 200, body = ObjectMetadata),
        (status = 404, description = "File not found")
    )
)]
pub async fn rename_file(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<RenameFileRequest>,
) -> Result<Json<ObjectMetadata>, HttpError> {
    let provider = state.registry().clone();
    let metadata = tokio::task::spawn_blocking(move || provider.rename_file(&id, request.filename))
    .await
    .map_err(|error| crate::error::LiveError::Conflict(format!("join file rename: {error}")))??;
    Ok(Json(metadata))
}
