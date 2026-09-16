use crate::app::AppState;
use crate::module::{LiveModule, ModuleDescriptor, OperationDescriptor};
use crate::modules::{registry, HttpError};
use crate::protocol::{operation, ObjectMetadata, ObjectPage, ObjectQuery, OperationKind};
use axum::body::{Body, Bytes};
use axum::extract::{Path, Query, State};
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
        id: operation::FILES_SEARCH,
        kind: OperationKind::Read,
        fallback_safe_before_dispatch: true,
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
            .route("/api/v1/files/search", get(search_files))
            .route("/api/v1/files/{id}", get(get_file).patch(rename_file))
    }

    fn openapi(&self) -> utoipa::openapi::OpenApi {
        <FilesApiDoc as utoipa::OpenApi>::openapi()
    }
}

#[derive(utoipa::OpenApi)]
#[openapi(
    paths(list_files, put_file, get_file, rename_file, search_files),
    components(schemas(ObjectMetadata, RenameFileRequest, ObjectPage, ObjectQuery)),
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
    get,
    path = "/api/v1/files/search",
    params(
        ("media_type" = Option<String>, Query, description = "Exact media type"),
        ("filename_contains" = Option<String>, Query, description = "Filename substring"),
        ("min_size" = Option<u64>, Query, description = "Minimum size in bytes"),
        ("max_size" = Option<u64>, Query, description = "Maximum size in bytes"),
        ("created_after_millis" = Option<u64>, Query, description = "Exclusive lower bound"),
        ("created_before_millis" = Option<u64>, Query, description = "Exclusive upper bound"),
        ("limit" = Option<u32>, Query, description = "Page size; defaults to 100, clamped to 1000"),
        ("cursor" = Option<String>, Query, description = "Opaque cursor from a previous page")
    ),
    responses((status = 200, body = ObjectPage))
)]
pub async fn search_files(
    State(state): State<AppState>,
    Query(mut query): Query<ObjectQuery>,
) -> Result<Json<ObjectPage>, HttpError> {
    // The files surface is the file-kind projection of the object surface, so
    // the kind is fixed here rather than trusted from the caller.
    query.kind = Some("file".to_owned());
    let registry = state.registry().clone();
    let page = tokio::task::spawn_blocking(move || registry.search(&query))
        .await
        .map_err(|error| {
            crate::error::LiveError::Conflict(format!("join file search: {error}"))
        })??;
    Ok(Json(page))
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
