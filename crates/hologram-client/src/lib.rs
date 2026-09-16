//! Typed HTTP client for the Hologram Live object and file API.
//!
//! This crate is deliberately standalone. It mirrors the wire shapes rather
//! than importing them from `hologram-live`, because depending on the daemon
//! would pull wasmtime, axum, and tonic into every consumer's build for the
//! sake of a handful of JSON structures.
//!
//! The cost of that choice is drift: these types can fall out of step with the
//! server's. A contract test in the daemon's own suite serializes the server
//! types and deserializes them here, so drift fails a build rather than
//! surfacing as a confusing runtime error.

#![forbid(unsafe_code)]

mod query;

pub use query::ObjectQuery;

use serde::{Deserialize, Serialize};

/// Metadata for one stored object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectMetadata {
    pub id: String,
    pub kind: String,
    pub media_type: String,
    pub filename: Option<String>,
    pub size: u64,
    pub created_at_millis: u64,
}

/// One page of search results.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectPage {
    pub objects: Vec<ObjectMetadata>,
    /// Absent when the walk reached the end of the result set.
    pub next_cursor: Option<String>,
    /// True when a provider-side scan bound stopped the walk early. A caller
    /// paginating to completion must not treat a truncated page as the end.
    pub truncated: bool,
}

/// A typed error from the daemon, or a transport failure reaching it.
#[derive(Debug)]
pub enum Error {
    /// The daemon answered with a typed error.
    Api { code: String, message: String },
    /// The request never produced an answer.
    Transport(String),
    /// The answer did not match the expected shape.
    Decode(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Api { code, message } => write!(formatter, "{code}: {message}"),
            Self::Transport(message) => write!(formatter, "transport: {message}"),
            Self::Decode(message) => write!(formatter, "decode: {message}"),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Deserialize)]
struct ApiError {
    code: String,
    message: String,
}

/// A client for one Hologram daemon.
pub struct HologramClient {
    http: reqwest::Client,
    base: String,
    token: Option<String>,
}

impl HologramClient {
    /// Build a client against `base_url`, for example `http://127.0.0.1:11435`.
    pub fn new(base_url: impl Into<String>) -> Result<Self> {
        // reqwest 0.13 requires an explicitly installed rustls provider, and
        // building a client without one panics. Installing it here means a
        // consumer of this crate does not have to know that.
        install_crypto_provider();
        let http = reqwest::Client::builder()
            .build()
            .map_err(|error| Error::Transport(format!("build client: {error}")))?;
        Ok(Self {
            http,
            base: base_url.into().trim_end_matches('/').to_owned(),
            token: None,
        })
    }

    /// Send `Authorization: Bearer` on every request.
    #[must_use]
    pub fn with_token(mut self, token: impl Into<String>) -> Self {
        let token = token.into();
        self.token = (!token.is_empty()).then_some(token);
        self
    }

    fn authorize(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match self.token.as_ref() {
            Some(token) => builder.bearer_auth(token),
            None => builder,
        }
    }

    /// Store a file and return its content-addressed metadata.
    pub async fn put_file(
        &self,
        bytes: Vec<u8>,
        filename: Option<&str>,
        media_type: &str,
    ) -> Result<ObjectMetadata> {
        let mut request = self
            .http
            .post(format!("{}/api/v1/files", self.base))
            .header(reqwest::header::CONTENT_TYPE, media_type)
            .body(bytes);
        if let Some(filename) = filename {
            request = request.header("x-hologram-filename", filename);
        }
        self.json(request).await
    }

    /// Fetch a file's bytes by content-addressed id.
    pub async fn get_file(&self, id: &str) -> Result<Vec<u8>> {
        let request = self.http.get(format!("{}/api/v1/files/{id}", self.base));
        let response = self.send(request).await?;
        response
            .bytes()
            .await
            .map(|body| body.to_vec())
            .map_err(|error| Error::Transport(format!("read body: {error}")))
    }

    pub async fn list_files(&self) -> Result<Vec<ObjectMetadata>> {
        self.json(self.http.get(format!("{}/api/v1/files", self.base)))
            .await
    }

    /// Search files. The daemon fixes the kind for this surface.
    pub async fn search_files(&self, query: &ObjectQuery) -> Result<ObjectPage> {
        let request = self
            .http
            .get(format!("{}/api/v1/files/search", self.base))
            .query(&query.to_pairs());
        self.json(request).await
    }

    /// Store an object of any kind.
    pub async fn put_object(
        &self,
        kind: &str,
        bytes: Vec<u8>,
        filename: Option<&str>,
        media_type: &str,
    ) -> Result<ObjectMetadata> {
        let mut request = self
            .http
            .post(format!("{}/api/v1/objects", self.base))
            .header(reqwest::header::CONTENT_TYPE, media_type)
            .header("x-hologram-kind", kind)
            .body(bytes);
        if let Some(filename) = filename {
            request = request.header("x-hologram-filename", filename);
        }
        self.json(request).await
    }

    pub async fn get_object(&self, id: &str) -> Result<Vec<u8>> {
        let request = self.http.get(format!("{}/api/v1/objects/{id}", self.base));
        let response = self.send(request).await?;
        response
            .bytes()
            .await
            .map(|body| body.to_vec())
            .map_err(|error| Error::Transport(format!("read body: {error}")))
    }

    pub async fn list_objects(&self) -> Result<Vec<ObjectMetadata>> {
        self.json(self.http.get(format!("{}/api/v1/objects", self.base)))
            .await
    }

    pub async fn search_objects(&self, query: &ObjectQuery) -> Result<ObjectPage> {
        let request = self
            .http
            .get(format!("{}/api/v1/objects/search", self.base))
            .query(&query.to_pairs());
        self.json(request).await
    }

    /// Walk every page of a search, following cursors to the end.
    ///
    /// Stops at a truncated page rather than looping: truncation means the
    /// daemon stopped early, so continuing would spin without progressing.
    pub async fn search_objects_all(&self, query: &ObjectQuery) -> Result<Vec<ObjectMetadata>> {
        let mut query = query.clone();
        let mut collected = Vec::new();
        loop {
            let page = self.search_objects(&query).await?;
            collected.extend(page.objects);
            match page.next_cursor {
                Some(cursor) if !page.truncated => query.cursor = Some(cursor),
                _ => return Ok(collected),
            }
        }
    }

    async fn send(&self, request: reqwest::RequestBuilder) -> Result<reqwest::Response> {
        let response = self
            .authorize(request)
            .send()
            .await
            .map_err(|error| Error::Transport(error.to_string()))?;
        if response.status().is_success() {
            return Ok(response);
        }
        // The daemon answers errors as a typed document, so surface its code
        // rather than a bare status a caller would have to guess about.
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        Err(match serde_json::from_str::<ApiError>(&body) {
            Ok(api) => Error::Api {
                code: api.code,
                message: api.message,
            },
            Err(_) => Error::Api {
                code: format!("HTTP_{}", status.as_u16()),
                message: body,
            },
        })
    }

    async fn json<T: for<'de> Deserialize<'de>>(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<T> {
        let response = self.send(request).await?;
        response
            .json()
            .await
            .map_err(|error| Error::Decode(error.to_string()))
    }
}

fn install_crypto_provider() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        // Errors only when a provider is already installed, which is exactly
        // the state this call exists to guarantee.
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}
