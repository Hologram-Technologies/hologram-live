use crate::app::AppState;
use crate::module::{LiveModule, ModuleDescriptor, OperationDescriptor};
use crate::modules::HttpError;
use crate::protocol::{operation, ObjectContent, ObjectMetadata, OperationKind};
use axum::body::{Body, Bytes};
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::Response;
use axum::routing::get;
use axum::{Json, Router};

const OPERATIONS: &[OperationDescriptor] = &[
    OperationDescriptor {
        id: operation::REGISTRY_LIST,
        kind: OperationKind::Read,
        fallback_safe_before_dispatch: true,
    },
    OperationDescriptor {
        id: operation::REGISTRY_GET,
        kind: OperationKind::Read,
        fallback_safe_before_dispatch: true,
    },
    OperationDescriptor {
        id: operation::REGISTRY_PUT,
        kind: OperationKind::Mutation,
        fallback_safe_before_dispatch: false,
    },
];

static DESCRIPTOR: ModuleDescriptor = ModuleDescriptor {
    id: "dev.hologram.live.kappa-registry",
    name: "Kappa Registry Provider",
    version: env!("CARGO_PKG_VERSION"),
    dependencies: &["dev.hologram.live.system"],
    operations: OPERATIONS,
};

pub struct KappaRegistryModule;

impl LiveModule for KappaRegistryModule {
    fn descriptor(&self) -> &'static ModuleDescriptor {
        &DESCRIPTOR
    }

    fn router(&self) -> Router<AppState> {
        Router::new()
            .route("/api/v1/objects", get(list_objects).post(put_object))
            .route("/api/v1/objects/{id}", get(get_object))
    }

    fn openapi(&self) -> utoipa::openapi::OpenApi {
        <RegistryApiDoc as utoipa::OpenApi>::openapi()
    }
}

#[derive(utoipa::OpenApi)]
#[openapi(
    paths(list_objects, put_object, get_object),
    components(schemas(ObjectMetadata)),
    tags((name = "kappa-registry", description = "Content-addressed registry provider"))
)]
struct RegistryApiDoc;

#[utoipa::path(
    get,
    path = "/api/v1/objects",
    responses((status = 200, body = [ObjectMetadata]))
)]
pub async fn list_objects(
    State(state): State<AppState>,
) -> Result<Json<Vec<ObjectMetadata>>, HttpError> {
    let registry = state.registry().clone();
    let objects = tokio::task::spawn_blocking(move || registry.list_objects(None))
        .await
        .map_err(|error| {
            crate::error::LiveError::Conflict(format!("join object listing: {error}"))
        })??;
    Ok(Json(objects))
}

#[utoipa::path(
    post,
    path = "/api/v1/objects",
    request_body(content = Vec<u8>, content_type = "application/octet-stream"),
    params(
        ("content-type" = Option<String>, Header, description = "Stored media type"),
        ("x-hologram-kind" = Option<String>, Header, description = "Object kind; defaults to file"),
        ("x-hologram-filename" = Option<String>, Header, description = "Original filename")
    ),
    responses((status = 201, body = ObjectMetadata))
)]
pub async fn put_object(
    State(state): State<AppState>,
    headers: HeaderMap,
    bytes: Bytes,
) -> Result<(StatusCode, Json<ObjectMetadata>), HttpError> {
    let kind = optional_header(&headers, "x-hologram-kind")?.unwrap_or_else(|| "file".to_owned());
    let media_type = optional_header(&headers, header::CONTENT_TYPE.as_str())?
        .unwrap_or_else(|| "application/octet-stream".to_owned());
    let filename = optional_header(&headers, "x-hologram-filename")?;
    let metadata = store_object(&state, kind, media_type, filename, bytes.to_vec()).await?;
    Ok((StatusCode::CREATED, Json(metadata)))
}

#[utoipa::path(
    get,
    path = "/api/v1/objects/{id}",
    params(("id" = String, Path, description = "Content-addressed object ID")),
    responses(
        (status = 200, description = "Raw object bytes", content_type = "application/octet-stream"),
        (status = 404, description = "Object not found")
    )
)]
pub async fn get_object(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, HttpError> {
    let registry = state.registry().clone();
    let object = tokio::task::spawn_blocking(move || registry.get_object(&id))
        .await
        .map_err(|error| {
            crate::error::LiveError::Conflict(format!("join object read: {error}"))
        })??;
    object_response(object)
}

pub async fn store_object(
    state: &AppState,
    kind: String,
    media_type: String,
    filename: Option<String>,
    bytes: Vec<u8>,
) -> Result<ObjectMetadata, HttpError> {
    let registry = state.registry().clone();
    tokio::task::spawn_blocking(move || registry.put_object(kind, media_type, filename, &bytes))
        .await
        .map_err(|error| crate::error::LiveError::Conflict(format!("join object write: {error}")))?
        .map_err(Into::into)
}

pub fn object_response(object: ObjectContent) -> Result<Response, HttpError> {
    let mut builder = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, &object.metadata.media_type)
        .header(header::ETAG, format!("\"{}\"", object.metadata.id));
    if let Some(filename) = object.metadata.filename.as_deref() {
        builder = builder.header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{}\"", safe_filename(filename)),
        );
    }
    builder.body(Body::from(object.bytes)).map_err(|error| {
        crate::error::LiveError::Protocol(format!("build object response: {error}")).into()
    })
}

pub(crate) fn optional_header(
    headers: &HeaderMap,
    name: &str,
) -> Result<Option<String>, HttpError> {
    headers
        .get(name)
        .map(|value| {
            value.to_str().map(str::to_owned).map_err(|error| {
                crate::error::LiveError::Protocol(format!("invalid {name} header: {error}")).into()
            })
        })
        .transpose()
}

fn safe_filename(filename: &str) -> String {
    let basename = filename.rsplit(['/', '\\']).next().unwrap_or(filename);
    let safe: String = basename
        .chars()
        .map(|character| match character {
            value if value.is_ascii_alphanumeric() => value,
            '.' | '-' | '_' | ' ' => character,
            _ => '_',
        })
        .collect();
    if safe.is_empty() {
        "download".to_owned()
    } else {
        safe
    }
}
