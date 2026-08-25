use crate::app::AppState;
use crate::auth::Principal;
use crate::error::{ApiError, LiveError, Result};
use crate::grpc;
use crate::module::ModuleRegistry;
use crate::protocol::HealthResponse;
use crate::util::constant_time_eq;
use axum::extract::{DefaultBodyLimit, Request, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use scalar_api_reference::{get_asset_with_mime, scalar_html};
use serde_json::json;
use std::sync::atomic::{AtomicU64, Ordering};
use tracing::Instrument;
use utoipa::openapi::OpenApi as OpenApiDocument;

static REQUEST_IDS: AtomicU64 = AtomicU64::new(1);

#[derive(utoipa::OpenApi)]
#[openapi(
    info(title = "Hologram Live API", version = "1.0.0"),
    paths(healthz),
    components(schemas(ApiError, HealthResponse)),
    tags((name = "system", description = "Hologram Live system endpoints"))
)]
pub struct ApiDoc;

pub fn openapi_document(modules: &ModuleRegistry) -> OpenApiDocument {
    let mut document = <ApiDoc as utoipa::OpenApi>::openapi();
    document.merge(modules.openapi());
    document
}

pub async fn serve(state: AppState) -> Result<()> {
    serve_with_ready(state, || Ok(())).await
}

/// Serve until shutdown, invoking `on_ready` only after the listener has
/// bound successfully. CLI frontends use this seam to emit an accurate JSON
/// readiness document without announcing a server that failed to bind.
pub async fn serve_with_ready<F>(state: AppState, on_ready: F) -> Result<()>
where
    F: FnOnce() -> Result<()>,
{
    let protected = state
        .module_router()
        .layer(middleware::from_fn_with_state(state.clone(), authenticate));
    let grpc = grpc::router(state.clone());

    let router = Router::new()
        .route("/", get(index))
        .route("/healthz", get(healthz))
        .route("/openapi.json", get(openapi))
        .route("/docs", get(scalar_reference))
        .route("/docs/scalar.js", get(scalar_javascript))
        .merge(protected)
        .with_state(state.clone())
        .merge(grpc)
        .layer(DefaultBodyLimit::max(
            state.config().server.max_http_body_bytes,
        ));

    let listener = tokio::net::TcpListener::bind(&state.config().server.listen)
        .await
        .map_err(|error| {
            LiveError::Transport(format!("bind {}: {error}", state.config().server.listen))
        })?;
    on_ready()?;
    tracing::info!(listen = %state.config().server.listen, "hologram server ready");
    let shutdown_state = state.clone();
    let result = axum::serve(listener, router)
        .with_graceful_shutdown(async move { shutdown_state.wait_shutdown().await })
        .await
        .map_err(|error| LiveError::Transport(format!("serve HTTP: {error}")));
    state.chat().engine().shutdown().await;
    state.plugins().shutdown().await;
    result
}

async fn index() -> Html<&'static str> {
    Html(
        r#"<!doctype html>
<html lang="en">
<head><meta charset="utf-8"><meta name="viewport" content="width=device-width"><title>Hologram</title>
<style>body{font-family:system-ui;margin:3rem;max-width:70rem}code{background:#eee;padding:.2rem .4rem}li{margin:.5rem 0}</style></head>
<body><h1>Hologram Live</h1><p>The local module host is running.</p>
<ul><li><a href="/healthz">Health</a></li><li><a href="/docs">API reference</a></li>
<li><a href="/openapi.json">Raw OpenAPI</a></li>
<li><a href="/api/v1/modules">Modules</a></li><li><a href="/api/v1/objects">Objects</a></li>
<li><a href="/api/v1/files">Files</a></li><li><a href="/api/v1/holo">.holo catalog</a></li></ul>
<p>Native clients use the <code>hologram.live.v1.HologramLive</code> gRPC service.</p></body></html>"#,
    )
}

#[utoipa::path(
    get,
    path = "/healthz",
    responses((status = 200, body = HealthResponse))
)]
pub async fn healthz(State(state): State<AppState>) -> Json<HealthResponse> {
    Json(state.health())
}

async fn openapi(State(state): State<AppState>) -> Json<OpenApiDocument> {
    Json(openapi_document(state.module_registry()))
}

async fn scalar_reference() -> Html<String> {
    let configuration = json!({
        "url": "/openapi.json",
        "layout": "modern",
        "theme": "default",
        "darkMode": true,
        "hideModels": false,
        "showSidebar": true,
        "agent": { "disabled": true }
    });
    Html(scalar_html(&configuration, Some("/docs/scalar.js")))
}

async fn scalar_javascript() -> Response {
    match get_asset_with_mime("scalar.js") {
        Some((content_type, content)) => Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, content_type)
            .header(header::CACHE_CONTROL, "public, max-age=86400")
            .body(axum::body::Body::from(content))
            .expect("valid Scalar JavaScript response"),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn authenticate(State(state): State<AppState>, mut request: Request, next: Next) -> Response {
    let request_id = REQUEST_IDS.fetch_add(1, Ordering::Relaxed);
    let span = tracing::info_span!(
        "live.server.request",
        request_id,
        method = %request.method(),
        path = %request.uri().path()
    );
    match principal_from_headers(&state, request.headers()) {
        Ok(principal) => {
            request.extensions_mut().insert(principal);
            next.run(request).instrument(span).await
        }
        Err(error) => {
            span.in_scope(|| tracing::warn!(code = error.code(), "request authentication failed"));
            let status = if matches!(error, LiveError::Authorization(_)) {
                StatusCode::FORBIDDEN
            } else {
                StatusCode::UNAUTHORIZED
            };
            (status, Json(ApiError::from(&error))).into_response()
        }
    }
}

fn principal_from_headers(state: &AppState, headers: &HeaderMap) -> Result<Principal> {
    if !state.config().auth.required {
        return Ok(Principal {
            id: "local-user".to_owned(),
            scope: "local".to_owned(),
        });
    }
    let configured = state.config().auth_token().ok_or_else(|| {
        LiveError::Authentication("server authentication token is unavailable".to_owned())
    })?;
    let supplied = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .ok_or_else(|| LiveError::Authentication("missing bearer token".to_owned()))?;
    if !constant_time_eq(configured.as_bytes(), supplied.as_bytes()) {
        return Err(LiveError::Authentication("invalid bearer token".to_owned()));
    }
    Ok(Principal {
        id: "token-principal".to_owned(),
        scope: "default".to_owned(),
    })
}
