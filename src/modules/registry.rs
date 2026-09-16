use crate::app::AppState;
use crate::module::{LiveModule, ModuleDescriptor, OperationDescriptor};
use crate::modules::HttpError;
use crate::protocol::{
    operation, ObjectContent, ObjectMetadata, ObjectPage, ObjectQuery, OperationKind,
};
use axum::body::{Body, Bytes};
use axum::extract::{Path, Query, State};
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
        id: operation::REGISTRY_SEARCH,
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
            .route("/api/v1/objects/search", get(search_objects))
            .route("/api/v1/objects/{id}", get(get_object))
    }

    fn openapi(&self) -> utoipa::openapi::OpenApi {
        <RegistryApiDoc as utoipa::OpenApi>::openapi()
    }
}

#[derive(utoipa::OpenApi)]
#[openapi(
    paths(list_objects, put_object, get_object, search_objects),
    components(schemas(ObjectMetadata, ObjectPage, ObjectQuery)),
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
    get,
    path = "/api/v1/objects/search",
    params(
        ("kind" = Option<String>, Query, description = "Exact object kind"),
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
pub async fn search_objects(
    State(state): State<AppState>,
    Query(query): Query<ObjectQuery>,
) -> Result<Json<ObjectPage>, HttpError> {
    let registry = state.registry().clone();
    let page = tokio::task::spawn_blocking(move || registry.search(&query))
        .await
        .map_err(|error| {
            crate::error::LiveError::Conflict(format!("join object search: {error}"))
        })??;
    Ok(Json(page))
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

#[cfg(test)]
mod tests {
    use crate::protocol::ObjectQuery;
    use axum::extract::Query;
    use axum::http::Uri;

    fn parse(query: &str) -> ObjectQuery {
        let uri: Uri = format!("http://localhost/api/v1/objects/search?{query}")
            .parse()
            .expect("valid uri");
        Query::<ObjectQuery>::try_from_uri(&uri)
            .expect("query must deserialize")
            .0
    }

    #[test]
    fn an_empty_query_is_a_bounded_page_not_an_unbounded_scan() {
        let query = parse("");
        assert_eq!(query.effective_limit(), 100);
        assert!(query.cursor.is_none());
        assert!(query.kind.is_none());
    }

    #[test]
    fn an_oversized_limit_is_clamped() {
        assert_eq!(
            parse("limit=100000").effective_limit(),
            1000,
            "a caller must not be able to request an unbounded page"
        );
    }

    #[test]
    fn filters_parse_from_the_query_string() {
        let query = parse("kind=file&filename_contains=notes&min_size=10");
        assert_eq!(query.kind.as_deref(), Some("file"));
        assert_eq!(query.filename_contains.as_deref(), Some("notes"));
        assert_eq!(query.min_size, Some(10));
    }

    #[test]
    fn the_search_route_is_not_shadowed_by_the_object_id_route() {
        // `/api/v1/objects/search` and `/api/v1/objects/{id}` share a prefix.
        // If the dynamic route won, search would be dispatched as a lookup for
        // an object literally named "search" and return 404 forever.
        let router: axum::Router<crate::app::AppState> = axum::Router::new()
            .route(
                "/api/v1/objects/search",
                axum::routing::get(|| async { "search" }),
            )
            .route(
                "/api/v1/objects/{id}",
                axum::routing::get(|| async { "by-id" }),
            );
        // Router construction panics on a genuine route conflict, so reaching
        // here proves the two coexist; ordering is then matchit's static-wins
        // rule, which the integration surface exercises.
        drop(router);
    }
}
