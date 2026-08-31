use hologram_live::actor::ActorSystem;
use hologram_live::audit::AuditLog;
use hologram_live::config::AppConfig;
use hologram_live::holo::HoloExecutor;
use hologram_live::holo_capability::EffectiveGrant;
use hologram_live::protocol::HoloRunResult;
use hologram_view_surface::{
    PortableViewAttachment, PortableViewIntentHandler, PortableViewSurface, SurfaceFuture,
    ViewAttachmentId, ViewIntentRequest, ViewSurfaceRegistry, MAX_INTENT_PAYLOAD_BYTES,
};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tauri::http::{header, Method, Request, Response, StatusCode};
use tauri::webview::NewWindowResponse;
use tauri::{AppHandle, Manager, Url, WebviewUrl, WebviewWindowBuilder};

pub const VIEW_SCHEME: &str = "hologram-view";

const VIEW_CSP: &str = concat!(
    "default-src 'self'; base-uri 'none'; connect-src 'self'; ",
    "form-action 'none'; frame-ancestors 'none'; object-src 'none'; ",
    "script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; ",
    "img-src 'self' data:; font-src 'self'"
);

type AssetError = (StatusCode, String);
type AssetLookup = Result<(String, Arc<[u8]>), AssetError>;

pub struct DesktopHoloRuntime {
    view_surfaces: Arc<ViewSurfaceRegistry>,
}

impl DesktopHoloRuntime {
    pub async fn execute(
        &self,
        bytes: &[u8],
        inputs: Vec<Vec<u8>>,
    ) -> Result<HoloRunResult, String> {
        let (config, _) = AppConfig::load(None).map_err(|error| error.to_string())?;
        config
            .create_directories()
            .map_err(|error| error.to_string())?;
        let actors = ActorSystem::start();
        let audit = AuditLog::open(
            config.paths.state_dir.join("audit.jsonl"),
            config.server.actor_mailbox_capacity,
            actors.root(),
        )
        .await
        .map_err(|error| error.to_string())?;
        HoloExecutor::with_view_surfaces(self.view_surfaces.clone())
            .execute_with_grant_and_audit(
                bytes,
                inputs,
                &EffectiveGrant::local_baseline(),
                &audit,
                "local-desktop",
            )
            .await
            .map_err(|error| error.to_string())
    }
}

pub fn initialize(
    app: &mut tauri::App,
    assets: Arc<ViewAssetStore>,
) -> Result<(), Box<dyn std::error::Error>> {
    let view_surfaces = Arc::new(ViewSurfaceRegistry::new());
    view_surfaces
        .register_portable(Arc::new(TauriViewSurface {
            app: app.handle().clone(),
            assets,
        }))
        .map_err(std::io::Error::other)?;
    app.manage(DesktopHoloRuntime { view_surfaces });
    Ok(())
}

pub struct ViewAssetStore {
    attachments: RwLock<HashMap<String, StoredAttachment>>,
}

impl Default for ViewAssetStore {
    fn default() -> Self {
        Self {
            attachments: RwLock::new(HashMap::new()),
        }
    }
}

struct StoredAttachment {
    id: ViewAttachmentId,
    token: String,
    entry: String,
    assets: HashMap<String, Arc<[u8]>>,
    intents: Arc<dyn PortableViewIntentHandler>,
}

struct StagedAttachment {
    token: String,
    label: String,
    entry: String,
}

impl ViewAssetStore {
    fn stage(&self, view: &PortableViewAttachment) -> Result<StagedAttachment, String> {
        validate_token(&view.id.token)?;
        let label = window_label(&view.id.token);
        let mut assets = HashMap::with_capacity(view.assets.len());
        for asset in &view.assets {
            validate_asset_path(&asset.path)?;
            if assets
                .insert(asset.path.clone(), asset.bytes.clone())
                .is_some()
            {
                return Err(format!("duplicate portable View asset {:?}", asset.path));
            }
        }
        if !assets.contains_key(&view.entry) {
            return Err(format!(
                "portable View entry {:?} is not present in its assets",
                view.entry
            ));
        }
        let stored = StoredAttachment {
            id: view.id.clone(),
            token: view.id.token.clone(),
            entry: view.entry.clone(),
            assets,
            intents: view.intents.clone(),
        };
        let mut attachments = self
            .attachments
            .write()
            .map_err(|_| "portable View asset store lock poisoned".to_owned())?;
        if attachments.contains_key(&label) {
            return Err(format!(
                "portable View attachment {} is already active",
                view.id.token
            ));
        }
        attachments.insert(label.clone(), stored);
        Ok(StagedAttachment {
            token: view.id.token.clone(),
            label,
            entry: view.entry.clone(),
        })
    }

    fn remove(&self, id: &ViewAttachmentId) -> Result<bool, String> {
        Ok(self
            .attachments
            .write()
            .map_err(|_| "portable View asset store lock poisoned".to_owned())?
            .remove(&window_label(&id.token))
            .is_some())
    }

    pub async fn response(
        &self,
        webview_label: &str,
        request: &Request<Vec<u8>>,
    ) -> Response<Vec<u8>> {
        if request.method() == Method::POST {
            return self.intent_response(webview_label, request).await;
        }
        match self.lookup(webview_label, request) {
            Ok((path, asset)) => asset_response(request.method(), &path, asset),
            Err((status, message)) => error_response(status, &message),
        }
    }

    async fn intent_response(
        &self,
        webview_label: &str,
        request: &Request<Vec<u8>>,
    ) -> Response<Vec<u8>> {
        if request.uri().scheme_str() != Some(VIEW_SCHEME)
            || request.uri().query().is_some()
            || request.uri().path() != "/_hologram/intent"
        {
            return error_response(StatusCode::BAD_REQUEST, "invalid portable View intent URL");
        }
        if request.body().len() > MAX_INTENT_PAYLOAD_BYTES + 1024 {
            return error_response(
                StatusCode::PAYLOAD_TOO_LARGE,
                "portable View intent request is too large",
            );
        }
        let content_type = request
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok());
        if !matches!(content_type, Some(value) if value.split(';').next() == Some("application/json"))
        {
            return error_response(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "portable View intents require application/json",
            );
        }
        let intent = match serde_json::from_slice::<ViewIntentRequest>(request.body()) {
            Ok(intent) => intent,
            Err(error) => {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    &format!("decode portable View intent: {error}"),
                );
            }
        };
        let (id, handler) = match self.intent_target(webview_label, request) {
            Ok(target) => target,
            Err((status, message)) => return error_response(status, &message),
        };
        match handler.handle(&id, intent).await {
            Ok(response) => match serde_json::to_vec(&response) {
                Ok(body) => response_builder(StatusCode::OK)
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::CONTENT_LENGTH, body.len())
                    .body(body)
                    .expect("portable View intent response headers are valid"),
                Err(error) => error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &format!("encode portable View intent response: {error}"),
                ),
            },
            Err(error) => error_response(StatusCode::UNPROCESSABLE_ENTITY, &error),
        }
    }

    fn intent_target(
        &self,
        webview_label: &str,
        request: &Request<Vec<u8>>,
    ) -> Result<(ViewAttachmentId, Arc<dyn PortableViewIntentHandler>), AssetError> {
        let attachments = self.attachments.read().map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "portable View asset store lock poisoned".to_owned(),
            )
        })?;
        let attachment = attachments.get(webview_label).ok_or_else(|| {
            (
                StatusCode::FORBIDDEN,
                "portable View attachment is unavailable".to_owned(),
            )
        })?;
        if request.uri().host() != Some(attachment.token.as_str()) {
            return Err((
                StatusCode::FORBIDDEN,
                "portable View origin does not match its attachment".to_owned(),
            ));
        }
        Ok((attachment.id.clone(), attachment.intents.clone()))
    }

    fn lookup(&self, webview_label: &str, request: &Request<Vec<u8>>) -> AssetLookup {
        if request.method() != Method::GET && request.method() != Method::HEAD {
            return Err((
                StatusCode::METHOD_NOT_ALLOWED,
                "method not allowed".to_owned(),
            ));
        }
        if request.uri().scheme_str() != Some(VIEW_SCHEME) || request.uri().query().is_some() {
            return Err((
                StatusCode::BAD_REQUEST,
                "invalid portable View URL".to_owned(),
            ));
        }
        let attachments = self.attachments.read().map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "portable View asset store lock poisoned".to_owned(),
            )
        })?;
        let attachment = attachments.get(webview_label).ok_or_else(|| {
            (
                StatusCode::FORBIDDEN,
                "portable View attachment is unavailable".to_owned(),
            )
        })?;
        if request.uri().host() != Some(attachment.token.as_str()) {
            return Err((
                StatusCode::FORBIDDEN,
                "portable View origin does not match its attachment".to_owned(),
            ));
        }
        let raw_path = request.uri().path();
        let path = if raw_path == "/" {
            attachment.entry.as_str()
        } else {
            raw_path.strip_prefix('/').unwrap_or(raw_path)
        };
        validate_asset_path(path).map_err(|error| (StatusCode::BAD_REQUEST, error))?;
        attachment
            .assets
            .get(path)
            .cloned()
            .map(|asset| (path.to_owned(), asset))
            .ok_or_else(|| {
                (
                    StatusCode::NOT_FOUND,
                    format!("portable View asset {path:?} was not found"),
                )
            })
    }
}

struct TauriViewSurface {
    app: AppHandle,
    assets: Arc<ViewAssetStore>,
}

impl PortableViewSurface for TauriViewSurface {
    fn attach(&self, view: PortableViewAttachment) -> SurfaceFuture<'_> {
        Box::pin(async move {
            let staged = self.assets.stage(&view)?;
            let url = attachment_url(&staged.token, &staged.entry)?;
            let expected_token = staged.token.clone();
            let build = WebviewWindowBuilder::new(
                &self.app,
                &staged.label,
                WebviewUrl::CustomProtocol(url),
            )
            .title("Hologram Application")
            .inner_size(900.0, 650.0)
            .min_inner_size(480.0, 360.0)
            .center()
            .use_https_scheme(true)
            .on_navigation(move |url| navigation_is_local(url, &expected_token))
            .on_new_window(|_, _| NewWindowResponse::Deny)
            .build();
            if let Err(error) = build {
                let _ = self.assets.remove(&view.id);
                return Err(format!("create portable View window: {error}"));
            }
            Ok(())
        })
    }

    fn detach<'a>(&'a self, id: &'a ViewAttachmentId) -> SurfaceFuture<'a> {
        Box::pin(async move {
            let label = window_label(&id.token);
            let removed = self.assets.remove(id)?;
            if let Some(window) = self.app.get_webview_window(&label) {
                window
                    .destroy()
                    .map_err(|error| format!("destroy portable View window: {error}"))?;
            } else if !removed {
                return Ok(());
            }
            Ok(())
        })
    }
}

fn attachment_url(token: &str, entry: &str) -> Result<Url, String> {
    Url::parse(&format!("{VIEW_SCHEME}://{token}/{entry}"))
        .map_err(|error| format!("construct portable View URL: {error}"))
}

fn navigation_is_local(url: &Url, token: &str) -> bool {
    if url.scheme() == VIEW_SCHEME {
        return url.host_str() == Some(token);
    }
    let rewritten_host = format!("{VIEW_SCHEME}.{token}");
    cfg!(any(windows, target_os = "android"))
        && matches!(url.scheme(), "http" | "https")
        && url.host_str() == Some(rewritten_host.as_str())
}

fn window_label(token: &str) -> String {
    format!("hologram-view-{token}")
}

fn validate_token(token: &str) -> Result<(), String> {
    if token.len() == 64
        && token
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err("portable View attachment token must be a 64-character hex digest".to_owned())
    }
}

fn validate_asset_path(path: &str) -> Result<(), String> {
    if path.is_empty()
        || path.starts_with('/')
        || path.ends_with('/')
        || path.contains('%')
        || path.contains('\\')
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(format!("invalid portable View asset path {path:?}"));
    }
    if !path
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/'))
    {
        return Err(format!("invalid portable View asset path {path:?}"));
    }
    Ok(())
}

fn asset_response(method: &Method, path: &str, asset: Arc<[u8]>) -> Response<Vec<u8>> {
    let body = if method == Method::HEAD {
        Vec::new()
    } else {
        asset.to_vec()
    };
    response_builder(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type(path))
        .header(header::CONTENT_LENGTH, asset.len())
        .body(body)
        .expect("static portable View response headers are valid")
}

fn error_response(status: StatusCode, message: &str) -> Response<Vec<u8>> {
    response_builder(status)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(message.as_bytes().to_vec())
        .expect("static portable View error headers are valid")
}

fn response_builder(status: StatusCode) -> tauri::http::response::Builder {
    Response::builder()
        .status(status)
        .header(header::CACHE_CONTROL, "no-store")
        .header(header::CONTENT_SECURITY_POLICY, VIEW_CSP)
        .header("x-content-type-options", "nosniff")
        .header("referrer-policy", "no-referrer")
}

fn content_type(path: &str) -> &'static str {
    let extension = path
        .rsplit_once('.')
        .map_or_else(String::new, |(_, extension)| extension.to_ascii_lowercase());
    match extension.as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" | "map" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "wasm" => "application/wasm",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hologram_view_surface::{
        IntentFuture, PortableViewAsset, ViewIntentResponse, APPLICATION_INVOKE_INTENT,
        VIEW_INTENT_VERSION,
    };

    struct EchoIntents;

    impl PortableViewIntentHandler for EchoIntents {
        fn handle<'a>(
            &'a self,
            _id: &'a ViewAttachmentId,
            request: ViewIntentRequest,
        ) -> IntentFuture<'a> {
            Box::pin(async move {
                request.validate()?;
                Ok(ViewIntentResponse {
                    version: VIEW_INTENT_VERSION,
                    outputs: vec![request.payload],
                })
            })
        }
    }

    fn attachment(token: char) -> PortableViewAttachment {
        PortableViewAttachment {
            id: ViewAttachmentId {
                token: token.to_string().repeat(64),
                application_kappa: "blake3:application".to_owned(),
                layer_position: 1,
            },
            entry: "index.html".to_owned(),
            assets: vec![
                PortableViewAsset {
                    path: "index.html".to_owned(),
                    bytes: Arc::from(b"<!doctype html><h1>view</h1>".as_slice()),
                },
                PortableViewAsset {
                    path: "assets/app.js".to_owned(),
                    bytes: Arc::from(b"console.log('view')".as_slice()),
                },
            ],
            intents: Arc::new(EchoIntents),
        }
    }

    fn request(method: Method, value: &str) -> Request<Vec<u8>> {
        Request::builder()
            .method(method)
            .uri(value)
            .body(Vec::new())
            .expect("request")
    }

    #[tokio::test]
    async fn assets_are_bound_to_the_opaque_origin_and_window() {
        let store = ViewAssetStore::default();
        let first = attachment('a');
        let second = attachment('b');
        let first_staged = store.stage(&first).expect("stage first");
        let second_staged = store.stage(&second).expect("stage second");

        let response = store
            .response(
                &first_staged.label,
                &request(Method::GET, &format!("{VIEW_SCHEME}://{}/", first.id.token)),
            )
            .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.body(), b"<!doctype html><h1>view</h1>");
        assert_eq!(
            response.headers()[header::CONTENT_SECURITY_POLICY],
            VIEW_CSP
        );
        let script = store
            .response(
                &first_staged.label,
                &request(
                    Method::HEAD,
                    &format!("{VIEW_SCHEME}://{}/assets/app.js", first.id.token),
                ),
            )
            .await;
        assert!(script.body().is_empty());
        assert_eq!(
            script.headers()[header::CONTENT_TYPE],
            "text/javascript; charset=utf-8"
        );

        let cross_origin = store
            .response(
                &first_staged.label,
                &request(
                    Method::GET,
                    &format!("{VIEW_SCHEME}://{}/index.html", second.id.token),
                ),
            )
            .await;
        assert_eq!(cross_origin.status(), StatusCode::FORBIDDEN);
        assert!(store.remove(&first.id).expect("remove"));
        assert_eq!(
            store
                .response(
                    &first_staged.label,
                    &request(
                        Method::GET,
                        &format!("{VIEW_SCHEME}://{}/index.html", first.id.token),
                    ),
                )
                .await
                .status(),
            StatusCode::FORBIDDEN
        );
        assert!(store.remove(&second.id).expect("remove second"));
        assert_eq!(second_staged.entry, "index.html");
    }

    #[test]
    fn navigation_stays_inside_one_attachment_origin() {
        let token = "d".repeat(64);
        assert!(navigation_is_local(
            &attachment_url(&token, "index.html").expect("url"),
            &token
        ));
        assert!(!navigation_is_local(
            &Url::parse("https://example.com/").expect("external url"),
            &token
        ));
    }

    #[tokio::test]
    async fn requests_reject_ambiguous_paths_methods_and_queries() {
        let store = ViewAssetStore::default();
        let view = attachment('c');
        let staged = store.stage(&view).expect("stage");
        let url = |path: &str| format!("{VIEW_SCHEME}://{}{path}", view.id.token);

        assert_eq!(
            store
                .response(&staged.label, &request(Method::POST, &url("/index.html")))
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            store
                .response(&staged.label, &request(Method::GET, &url("/%2e%2e/secret")))
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            store
                .response(
                    &staged.label,
                    &request(Method::GET, &url("/index.html?x=1"))
                )
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            store
                .response(&staged.label, &request(Method::GET, &url("/missing.css")))
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
    }

    #[tokio::test]
    async fn intent_endpoint_accepts_only_the_bounded_versioned_message() {
        let store = ViewAssetStore::default();
        let view = attachment('e');
        let staged = store.stage(&view).expect("stage");
        let url = format!("{VIEW_SCHEME}://{}/_hologram/intent", view.id.token);
        let body = serde_json::to_vec(&ViewIntentRequest {
            version: VIEW_INTENT_VERSION,
            name: APPLICATION_INVOKE_INTENT.to_owned(),
            payload: "hello view".to_owned(),
        })
        .expect("intent");
        let intent_request = Request::builder()
            .method(Method::POST)
            .uri(&url)
            .header(header::CONTENT_TYPE, "application/json")
            .body(body)
            .expect("request");
        let response = store.response(&staged.label, &intent_request).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            serde_json::from_slice::<ViewIntentResponse>(response.body()).expect("response"),
            ViewIntentResponse {
                version: VIEW_INTENT_VERSION,
                outputs: vec!["hello view".to_owned()]
            }
        );

        let wrong_origin = request(
            Method::GET,
            &format!("{VIEW_SCHEME}://{}/index.html", "f".repeat(64)),
        );
        assert_eq!(
            store.response(&staged.label, &wrong_origin).await.status(),
            StatusCode::FORBIDDEN
        );
    }
}
