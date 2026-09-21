//! Hologram Registry: the Docker Registry HTTP API v2 and the OCI Distribution
//! API, under `/v2/`, on the Kappa store (`apps/registry`).
//!
//! The module is opt-in, and it authenticates itself (ADR 026): the registry
//! protocol has its own challenge and its own error shape, so these routes are
//! mounted beside the server's bearer layer, not under it.
//!
//! No Kappa type is named here. Storage is `crate::oci_store`.

mod blobs;
mod body;
mod delete;
pub mod error;
mod listing;
mod manifests;
mod media;
mod openapi;
pub mod path;
mod respond;
mod uploads;

use crate::app::AppState;
use crate::module::{LiveModule, ModuleDescriptor};
use crate::oci_store::OciStore;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::header::{ALLOW, CONTENT_LENGTH, CONTENT_TYPE, LOCATION};
use axum::http::{HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use axum::Router;
use error::{ErrorCode, OciError};
use path::{BlobVerb, ManifestVerb, Route, UploadVerb};
use std::sync::Arc;
use tracing::Instrument;

pub const MODULE_ID: &str = "dev.hologram.live.oci";

static DESCRIPTOR: ModuleDescriptor = ModuleDescriptor {
    id: MODULE_ID,
    name: "Hologram Registry",
    version: env!("CARGO_PKG_VERSION"),
    dependencies: &["dev.hologram.live.system"],
    // The registry API is HTTP only: it adds no operation to the RPC surface.
    operations: &[],
};

/// What the registry serves from, and how it is set to behave.
#[derive(Clone)]
pub struct Registry {
    pub store: Arc<OciStore>,
    pub settings: Settings,
    /// The server's audit log. The registry's writes go into it as the
    /// server's own writes do. `None` in tests that do not look at it.
    pub audit: Option<crate::audit::AuditLog>,
}

/// Who a registry write is recorded as, until the registry's own login lands.
const PRINCIPAL: &str = "anonymous";

/// The reference's settings that change what a route answers. P5 reads them
/// from `config.yml` too; the environment names are the reference's own.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    /// `storage.delete.enabled`. Off by default, as in the reference.
    pub delete_enabled: bool,
    /// `http.headers`: added to every answer under `/v2/`, errors and
    /// preflights included. It is the reference's recipe for CORS, which a web
    /// UI on another origin needs (FR-R31): `Access-Control-Allow-Origin`,
    /// `-Methods`, `-Headers`, and `-Expose-Headers` naming
    /// `Docker-Content-Digest` and `Link`. P5 fills it from `config.yml`.
    pub headers: Vec<(axum::http::HeaderName, HeaderValue)>,
}

impl Settings {
    pub fn from_environment() -> Self {
        let on = |name: &str| {
            std::env::var(name).is_ok_and(|value| value.trim().eq_ignore_ascii_case("true"))
        };
        Self {
            delete_enabled: on("REGISTRY_STORAGE_DELETE_ENABLED"),
            headers: Vec::new(),
        }
    }
}

pub struct OciRegistryModule;

impl LiveModule for OciRegistryModule {
    fn descriptor(&self) -> &'static ModuleDescriptor {
        &DESCRIPTOR
    }

    fn router(&self) -> Router<AppState> {
        // No `.fallback(…)`: tonic's router carries one and the merge would
        // collide. The catch-all route makes it unnecessary.
        Router::new()
            .route("/v2", any(without_the_slash))
            .route("/v2/", any(dispatch))
            .route("/v2/{*rest}", any(dispatch))
            .layer(middleware::from_fn(envelope))
    }

    fn authenticates_itself(&self) -> bool {
        true
    }

    /// Into the server's one document: `/openapi.json` and `/docs` exist already.
    fn openapi(&self) -> utoipa::openapi::OpenApi {
        openapi::document()
    }
}

/// What the server's `authenticate` layer does for every other module, minus
/// the bearer check: a request id and a request span. Then the header every
/// response under `/v2/` carries, errors included.
async fn envelope(request: Request, next: Next) -> Response {
    let span = tracing::info_span!(
        "live.server.request",
        request_id = crate::server::next_request_id(),
        method = %request.method(),
        path = %request.uri().path()
    );
    let mut response = next.run(request).instrument(span).await;
    respond::stamp_version(response.headers_mut());
    response
}

/// `/v2` is not the base route; the reference's router redirects to it.
async fn without_the_slash() -> Response {
    // Byte for byte what the reference's router sends (gate B, `base`).
    let mut response = (StatusCode::MOVED_PERMANENTLY, "<a href=\"/v2/\">Moved Permanently</a>.\n\n").into_response();
    let headers = response.headers_mut();
    headers.insert(LOCATION, HeaderValue::from_static("/v2/"));
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"));
    response
}

async fn dispatch(State(state): State<AppState>, request: Request) -> Response {
    match state.oci_store() {
        Some(store) => {
            static SETTINGS: std::sync::OnceLock<Settings> = std::sync::OnceLock::new();
            let registry = Registry {
                store: store.clone(),
                settings: SETTINGS.get_or_init(Settings::from_environment).clone(),
                audit: Some(state.audit().clone()),
            };
            handle(registry, request).await
        }
        // The volume is opened whenever the module is enabled, so this is a
        // defect, not a state a client can cause.
        None => OciError::internal(&"the registry module is enabled without a volume").into_response(),
    }
}

/// Answer one request under `/v2/`.
pub async fn handle(registry: Registry, request: Request) -> Response {
    // The body is not `Sync`, so nothing borrowed from the whole request may
    // live across an await: the head is borrowed, the body is moved.
    let (head, body) = request.into_parts();
    let origin = origin(&head.headers);
    let configured = registry.settings.headers.clone();
    let audit = registry.audit.clone();
    let rest = head.uri.path().strip_prefix("/v2/").unwrap_or_default();
    let route = path::decode(rest)
        .ok_or_else(OciError::unknown_route)
        .and_then(|rest| path::parse(&head.method, &rest));
    let (mut response, record) = match route {
        Ok(route) => {
            let audited = audited(&route);
            let result = serve(registry, route, &head, body).await;
            let outcome = match &result {
                Ok(_) => "accepted".to_owned(),
                Err(error) => format!("error:{}", error.code().map_or("NOT_FOUND", ErrorCode::as_str)),
            };
            let response = result.unwrap_or_else(IntoResponse::into_response);
            let record = audited.and_then(|audited| audited.resolve(&response, outcome));
            (response, record)
        }
        Err(error) => (error.into_response(), None),
    };
    if let (Some(audit), Some((operation, resource, outcome))) = (audit, record) {
        let event = crate::audit::AuditEvent::new(PRINCIPAL, operation, Some(resource), outcome);
        if let Err(error) = audit.record(event).await {
            tracing::error!(%error, "failed to record a registry audit event");
        }
    }
    absolute_location(&mut response, origin.as_deref());
    // Last, so they are on errors and on `OPTIONS` too. A browser's preflight
    // carries no credentials: when login lands (P6) `OPTIONS` stays in front
    // of it, or no web UI on another origin can reach the registry.
    for (name, value) in configured {
        response.headers_mut().append(name, value);
    }
    response
}

/// A write the audit log records: the operation, and what it acted on.
struct Audited {
    operation: &'static str,
    resource: String,
    /// `POST …/uploads/` writes only when it answers 201 (a one-request
    /// upload, or a mount); a 202 opens a session and changes nothing.
    only_when_created: bool,
}

impl Audited {
    /// The record, or `None` for a request that turned out to change nothing.
    fn resolve(self, response: &Response, outcome: String) -> Option<(&'static str, String, String)> {
        let created = response.status() == StatusCode::CREATED;
        if self.only_when_created && !created && outcome == "accepted" {
            return None;
        }
        // A blob's digest is known only once it is stored: read it from the answer.
        let resource = match response
            .headers()
            .get(LOCATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|location| location.rsplit_once("/blobs/"))
        {
            Some((_, digest)) if created && !digest.starts_with("uploads/") => {
                format!("{}@{digest}", self.resource)
            }
            _ => self.resource,
        };
        Some((self.operation, resource, outcome))
    }
}

/// The routes that write, named as the server names its operations. Reads are
/// not recorded, as the server records none of its own.
fn audited(route: &Route) -> Option<Audited> {
    let (operation, resource, only_when_created) = match route {
        Route::Manifest {
            repo,
            reference,
            verb: ManifestVerb::Put,
        } => ("oci.manifest.put", format!("{repo}:{reference}"), false),
        Route::Manifest {
            repo,
            reference,
            verb: ManifestVerb::Delete,
        } => ("oci.manifest.delete", format!("{repo}:{reference}"), false),
        Route::Blob {
            repo,
            digest,
            verb: BlobVerb::Delete,
        } => ("oci.blob.delete", format!("{repo}@{digest}"), false),
        Route::Upload {
            repo,
            verb: UploadVerb::Put,
            ..
        } => ("oci.blob.put", repo.to_string(), false),
        Route::UploadStart { repo } => ("oci.blob.put", repo.to_string(), true),
        _ => return None,
    };
    Some(Audited {
        operation,
        resource,
        only_when_created,
    })
}

/// `scheme://host` as the client addressed us. `None` without a `Host`.
fn origin(headers: &axum::http::HeaderMap) -> Option<String> {
    let host = headers.get(axum::http::header::HOST)?.to_str().ok()?;
    let scheme = headers
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .filter(|scheme| *scheme == "https" || *scheme == "http")
        .unwrap_or("http");
    Some(format!("{scheme}://{host}"))
}

/// The reference answers `Location` as an absolute URL unless
/// `http.relativeurls` is set (gate B, every push scenario). The routes build
/// paths; this makes them what the reference sends.
fn absolute_location(response: &mut Response, origin: Option<&str>) {
    let Some(origin) = origin else { return };
    let absolute = response
        .headers()
        .get(LOCATION)
        .and_then(|value| value.to_str().ok())
        .filter(|path| path.starts_with('/'))
        .and_then(|path| HeaderValue::from_str(&format!("{origin}{path}")).ok());
    if let Some(absolute) = absolute {
        response.headers_mut().insert(LOCATION, absolute);
    }
}

async fn serve(
    registry: Registry,
    route: Route,
    head: &axum::http::request::Parts,
    body: Body,
) -> Result<Response, OciError> {
    let (headers, query) = (&head.headers, head.uri.query());
    let Registry {
        store, settings, ..
    } = registry;
    match route {
        Route::Base => Ok(json(StatusCode::OK, "{}")),
        Route::Options { allow } => {
            let mut response = Response::new(Body::empty());
            response
                .headers_mut()
                .insert(ALLOW, HeaderValue::from_static(allow));
            Ok(response)
        }
        Route::Blob {
            repo,
            digest,
            verb: verb @ (BlobVerb::Get | BlobVerb::Head),
        } => blobs::get(store, repo, digest, headers, verb == BlobVerb::Head).await,
        Route::Manifest {
            repo,
            reference,
            verb: verb @ (ManifestVerb::Get | ManifestVerb::Head),
        } => manifests::get(store, repo, reference, headers, verb == ManifestVerb::Head).await,
        Route::Manifest {
            repo,
            reference,
            verb: ManifestVerb::Put,
        } => manifests::put(store, repo, reference, headers, body).await,
        Route::UploadStart { repo } => uploads::start(store, repo, query, body).await,
        Route::Upload { repo, id, verb } => match verb {
            UploadVerb::Status => uploads::status(store, repo, id).await,
            UploadVerb::Patch => uploads::patch(store, repo, id, headers, body).await,
            UploadVerb::Put => uploads::put(store, repo, id, query, body).await,
            UploadVerb::Delete => uploads::cancel(store, repo, id).await,
        },
        Route::Catalog => listing::catalog(store, query).await,
        Route::TagsList { repo } => listing::tags(store, repo, query).await,
        Route::Referrers { repo, digest } => listing::referrers(store, repo, digest, query).await,
        // Off unless `storage.delete.enabled`, as in the reference.
        Route::Manifest { .. } | Route::Blob { .. } if !settings.delete_enabled => {
            Err(OciError::new(ErrorCode::Unsupported))
        }
        Route::Manifest {
            repo, reference, ..
        } => delete::manifest(store, repo, reference).await,
        Route::Blob { repo, digest, .. } => delete::blob(store, repo, digest).await,
    }
}

fn json(status: StatusCode, body: &'static str) -> Response {
    let mut response = Response::new(Body::from(body));
    *response.status_mut() = status;
    let headers = response.headers_mut();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(CONTENT_LENGTH, HeaderValue::from(body.len()));
    response
}
