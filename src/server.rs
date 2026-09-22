use crate::admin_socket as admin;
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

/// The next request id, for a module that builds its own request span
/// because it is mounted outside `authenticate`.
#[cfg_attr(
    not(feature = "oci"),
    allow(dead_code, reason = "only the opt-in registry module calls it")
)]
pub(crate) fn next_request_id() -> u64 {
    REQUEST_IDS.fetch_add(1, Ordering::Relaxed)
}

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
    let routers = state.module_routers();
    let protected = routers
        .protected
        .layer(middleware::from_fn_with_state(state.clone(), authenticate));
    let grpc = grpc::router(state.clone());
    let limit = state.config().server.max_http_body_bytes;

    let registry_mode = state.config().registry_mode;
    let front = if registry_mode {
        Router::new().route("/", get(registry_index))
    } else {
        Router::new().route("/", get(index))
    };
    let public = front
        .route("/healthz", get(healthz))
        .route("/openapi.json", get(openapi))
        .route("/docs", get(scalar_reference))
        .route("/docs/scalar.js", get(scalar_javascript))
        .merge(routers.open);

    // A certificate that cannot be read stops the start before anything binds.
    #[cfg(feature = "oci")]
    let tls: PublicTls = state
        .config()
        .tls
        .as_ref()
        .map(crate::tls::acceptor)
        .transpose()?;
    #[cfg(not(feature = "oci"))]
    let tls: PublicTls = None;
    // Registry mode binds its socket first: once the public port accepts,
    // administration is up and owner-only.
    let admin_listener = if registry_mode {
        Some(admin::bind(&state.config().admin_socket())?)
    } else {
        None
    };
    let listener = tokio::net::TcpListener::bind(&state.config().server.listen)
        .await
        .map_err(|error| {
            LiveError::Transport(format!("bind {}: {error}", state.config().server.listen))
        });
    let result = if let Some(admin_listener) = admin_listener {
        // ADR 028: the public port carries /v2/ and the public pages only;
        // the module API and gRPC, shutdown included, live on the socket.
        let public = assemble(public.with_state(state.clone()), Router::new(), limit);
        let admin = assemble(protected.with_state(state.clone()), grpc, limit);
        let served = match listener.and_then(|listener| on_ready().map(|()| listener)) {
            Ok(listener) => {
                tracing::info!(listen = %state.config().server.listen, "hologram registry ready");
                serve_registry_mode(&state, (listener, public, tls), (admin_listener, admin)).await
            }
            Err(error) => Err(error),
        };
        admin::remove(&state.config().admin_socket());
        served
    } else {
        let listener = listener?;
        let http = public.merge(protected).with_state(state.clone());
        let router = assemble(http, grpc, limit);
        on_ready()?;
        tracing::info!(listen = %state.config().server.listen, "hologram server ready");
        let shutdown_state = state.clone();
        axum::serve(listener, router)
            .with_graceful_shutdown(async move { shutdown_state.wait_shutdown().await })
            .await
            .map_err(|error| LiveError::Transport(format!("serve HTTP: {error}")))
    };
    state.chat().engine().shutdown().await;
    state.plugins().shutdown().await;
    let audit = state.audit().flush().await;
    match (result, audit) {
        (Err(error), _) | (Ok(()), Err(error)) => Err(error),
        (Ok(()), Ok(())) => Ok(()),
    }
}

/// The public listener's TLS, when `http.tls` is set.
#[cfg(feature = "oci")]
type PublicTls = Option<tokio_rustls::TlsAcceptor>;
#[cfg(not(feature = "oci"))]
type PublicTls = Option<std::convert::Infallible>;

/// Both listeners of registry mode, drained by the one shutdown signal. Either
/// failing ends the process with its error.
async fn serve_registry_mode(
    state: &AppState,
    (public_listener, public, tls): (tokio::net::TcpListener, Router, PublicTls),
    (admin_listener, admin): (admin::Listener, Router),
) -> Result<()> {
    let public_state = state.clone();
    #[cfg(feature = "oci")]
    let public_state_drain = state.config().server.graceful_shutdown_secs;
    let public = async move {
        let shutdown = async move { public_state.wait_shutdown().await };
        #[cfg(feature = "oci")]
        if let Some(acceptor) = tls {
            let drain = std::time::Duration::from_secs(public_state_drain);
            return crate::tls::serve(public_listener, acceptor, public, shutdown, drain).await;
        }
        #[cfg(not(feature = "oci"))]
        let _ = tls;
        axum::serve(public_listener, public)
            .with_graceful_shutdown(shutdown)
            .await
            .map_err(|error| LiveError::Transport(format!("serve HTTP: {error}")))
    };
    let admin = admin::serve(state.clone(), admin_listener, admin);
    tokio::try_join!(public, admin).map(|_| ())
}

/// Join the HTTP routes and the gRPC service into the one router the listener serves.
fn assemble(http: Router, grpc: Router, max_http_body_bytes: usize) -> Router {
    http.merge(grpc)
        .fallback(no_route)
        .layer(DefaultBodyLimit::max(max_http_body_bytes))
}

/// Anything no route claims.
///
/// tonic's router carries its own catch-all, which answers every unknown path
/// with `200 application/grpc` and `grpc-status: 12`. Merged into a router
/// that has none, it became the whole server's fallback: a mistyped REST
/// path, a browser preflight and a metrics scrape all read as successes.
/// A gRPC caller still gets that answer; everyone else gets a 404 in the
/// daemon's error envelope.
async fn no_route(request: Request) -> Response {
    let grpc = request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("application/grpc"));
    if grpc {
        return tonic::Status::unimplemented("").into_http();
    }
    crate::modules::HttpError(LiveError::NotFound(format!(
        "no route for {}",
        request.uri().path()
    )))
    .into_response()
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
    if state.config().registry_mode {
        // Only what the public port serves: the module API is on the socket.
        let mut document = <ApiDoc as utoipa::OpenApi>::openapi();
        document.merge(state.module_registry().open_openapi());
        return Json(document);
    }
    Json(openapi_document(state.module_registry()))
}

/// Registry mode's `/`: what this port serves, and nothing it does not.
async fn registry_index() -> Html<&'static str> {
    Html(
        r#"<!doctype html>
<html lang="en">
<head><meta charset="utf-8"><title>Hologram Registry</title></head>
<body>
<h1>Hologram Registry</h1>
<p>A Docker Registry: the distribution API is at <a href="/v2/">/v2/</a>.
The API reference is at <a href="/docs">/docs</a>; health at <a href="/healthz">/healthz</a>.</p>
</body>
</html>
"#,
    )
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
    let request_id = next_request_id();
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use tower::ServiceExt;

    /// The same shape `serve_with_ready` builds: some HTTP routes, and tonic's router, which brings its own catch-all.
    fn router() -> Router {
        let http = Router::new().route("/healthz", get(|| async { "ok" }));
        let grpc = tonic::service::Routes::default().into_axum_router();
        assemble(http, grpc, 1024)
    }

    async fn send(method: &str, path: &str, content_type: Option<&str>) -> Response {
        let mut request = Request::builder().method(method).uri(path);
        if let Some(value) = content_type {
            request = request.header(header::CONTENT_TYPE, value);
        }
        router()
            .oneshot(request.body(Body::empty()).expect("request"))
            .await
            .expect("infallible")
    }

    #[tokio::test]
    async fn an_unknown_path_is_a_json_404_not_a_grpc_200() {
        let response = send("GET", "/api/v1/nope", None).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok()),
            Some("application/json")
        );
        let body = to_bytes(response.into_body(), 4096).await.expect("body");
        let error: serde_json::Value =
            serde_json::from_slice(&body).expect("the daemon's error envelope");
        assert_eq!(error["code"], "LIVE_NOT_FOUND");
    }

    #[tokio::test]
    async fn a_preflight_on_an_unknown_path_is_not_answered_as_grpc() {
        let response = send("OPTIONS", "/metrics", None).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn a_grpc_caller_still_gets_unimplemented_for_an_unknown_service() {
        let response = send("POST", "/other.Service/Method", Some("application/grpc")).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get("grpc-status")
                .and_then(|v| v.to_str().ok()),
            Some("12"),
            "UNIMPLEMENTED, as tonic answers it"
        );
    }

    #[tokio::test]
    async fn a_known_route_is_untouched() {
        assert_eq!(send("GET", "/healthz", None).await.status(), StatusCode::OK);
    }
}
