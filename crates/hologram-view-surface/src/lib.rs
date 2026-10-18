//! Host-neutral attachment boundary for portable Hologram View layers.

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, RwLock};

pub const PORTABLE_SURFACE: &str = "portable";
pub const VIEW_INTENT_VERSION: u16 = 1;
pub const APPLICATION_INVOKE_INTENT: &str = "application.invoke";
pub const MAX_INTENT_PAYLOAD_BYTES: usize = 64 * 1024;
pub const MAX_INTENT_OUTPUTS: usize = 16;
pub const MAX_INTENT_OUTPUT_BYTES: usize = 1024 * 1024;
/// Explicit larger bounded text transport; version 1 limits remain unchanged.
pub const VIEW_INTENT_VERSION_2: u16 = 2;
pub const MAX_INTENT_V2_PAYLOAD_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_INTENT_V2_OUTPUT_BYTES: usize = 4 * 1024 * 1024;

/// Immutable decoded UTF-8 byte budgets selected by the explicit wire version.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ViewIntentLimits {
    pub payload_bytes: usize,
    pub output_bytes: usize,
}

/// Select a closed transport version without granting another intent or host capability.
pub fn intent_limits(version: u16) -> Result<ViewIntentLimits, String> {
    match version {
        VIEW_INTENT_VERSION => Ok(ViewIntentLimits {
            payload_bytes: MAX_INTENT_PAYLOAD_BYTES,
            output_bytes: MAX_INTENT_OUTPUT_BYTES,
        }),
        VIEW_INTENT_VERSION_2 => Ok(ViewIntentLimits {
            payload_bytes: MAX_INTENT_V2_PAYLOAD_BYTES,
            output_bytes: MAX_INTENT_V2_OUTPUT_BYTES,
        }),
        _ => Err(format!(
            "unsupported portable View intent version {version}; expected 1 or 2"
        )),
    }
}

pub type SurfaceFuture<'a> = Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>>;
pub type IntentFuture<'a> =
    Pin<Box<dyn Future<Output = Result<ViewIntentResponse, String>> + Send + 'a>>;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ViewAttachmentId {
    pub token: String,
    pub application_kappa: String,
    pub layer_position: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct ViewIntentRequest {
    pub version: u16,
    pub name: String,
    pub payload: String,
}

impl ViewIntentRequest {
    pub fn validate(&self) -> Result<(), String> {
        let limits = intent_limits(self.version)?;
        if self.name != APPLICATION_INVOKE_INTENT {
            return Err(format!(
                "unsupported portable View intent {:?}; expected {APPLICATION_INVOKE_INTENT:?}",
                self.name
            ));
        }
        if self.payload.len() > limits.payload_bytes {
            return Err(format!(
                "portable View intent payload is {} bytes; maximum is {}",
                self.payload.len(),
                limits.payload_bytes
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct ViewIntentResponse {
    pub version: u16,
    pub outputs: Vec<String>,
}

pub trait PortableViewIntentHandler: Send + Sync {
    fn handle<'a>(
        &'a self,
        id: &'a ViewAttachmentId,
        request: ViewIntentRequest,
    ) -> IntentFuture<'a>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PortableViewAsset {
    pub path: String,
    pub bytes: Arc<[u8]>,
}

#[derive(Clone)]
pub struct PortableViewAttachment {
    pub id: ViewAttachmentId,
    pub entry: String,
    pub assets: Vec<PortableViewAsset>,
    pub intents: Arc<dyn PortableViewIntentHandler>,
}

impl fmt::Debug for PortableViewAttachment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PortableViewAttachment")
            .field("id", &self.id)
            .field("entry", &self.entry)
            .field("assets", &self.assets)
            .field("intents", &"<bounded handler>")
            .finish()
    }
}

/// One trusted host surface capable of attaching a portable View bundle.
///
/// Implementations own all platform types and authority. View content receives
/// only the assets in [`PortableViewAttachment`], never the host adapter.
pub trait PortableViewSurface: Send + Sync {
    fn attach(&self, view: PortableViewAttachment) -> SurfaceFuture<'_>;
    fn detach<'a>(&'a self, id: &'a ViewAttachmentId) -> SurfaceFuture<'a>;
}

/// Dynamically publishes the host's currently available portable surface.
///
/// Prepared layers retain the resolved `Arc`, so replacing or clearing the
/// registry cannot redirect an attachment midway through its lifecycle.
#[derive(Default)]
pub struct ViewSurfaceRegistry {
    portable: RwLock<Option<Arc<dyn PortableViewSurface>>>,
}

impl ViewSurfaceRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_portable(&self, surface: Arc<dyn PortableViewSurface>) -> Result<(), String> {
        *self
            .portable
            .write()
            .map_err(|_| "portable View surface registry lock poisoned".to_owned())? =
            Some(surface);
        Ok(())
    }

    pub fn unregister_portable(&self) -> Result<Option<Arc<dyn PortableViewSurface>>, String> {
        Ok(self
            .portable
            .write()
            .map_err(|_| "portable View surface registry lock poisoned".to_owned())?
            .take())
    }

    pub fn portable(&self) -> Result<Option<Arc<dyn PortableViewSurface>>, String> {
        Ok(self
            .portable
            .read()
            .map_err(|_| "portable View surface registry lock poisoned".to_owned())?
            .clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Surface;

    impl PortableViewSurface for Surface {
        fn attach(&self, _view: PortableViewAttachment) -> SurfaceFuture<'_> {
            Box::pin(async { Ok(()) })
        }

        fn detach<'a>(&'a self, _id: &'a ViewAttachmentId) -> SurfaceFuture<'a> {
            Box::pin(async { Ok(()) })
        }
    }

    #[test]
    fn registry_publishes_and_clears_one_portable_surface() {
        let registry = ViewSurfaceRegistry::new();
        assert!(registry.portable().expect("registry").is_none());
        registry
            .register_portable(Arc::new(Surface))
            .expect("register");
        assert!(registry.portable().expect("registry").is_some());
        assert!(registry.unregister_portable().expect("clear").is_some());
        assert!(registry.portable().expect("registry").is_none());
    }

    #[test]
    fn intent_v2_preserves_v1_and_admits_exact_larger_payloads() {
        let mut request = ViewIntentRequest {
            version: 2,
            name: APPLICATION_INVOKE_INTENT.to_owned(),
            payload: "x".repeat(2 * 1024 * 1024),
        };
        assert!(
            request.validate().is_ok(),
            "explicit intent2 must admit its exact two MiB boundary"
        );
        request.payload.push('x');
        assert!(request
            .validate()
            .expect_err("intent2 overflow")
            .contains("maximum"));
        request.payload = "🌱".repeat(512 * 1024);
        assert!(request.validate().is_ok(), "version2 counts UTF-8 bytes");
        request.payload.push('x');
        assert!(request.validate().is_err());
        request.version = 1;
        request.payload = "x".repeat(MAX_INTENT_PAYLOAD_BYTES);
        assert!(request.validate().is_ok());
        request.payload.push('x');
        assert!(request
            .validate()
            .expect_err("legacy cap")
            .contains("maximum"));
        request.version = 3;
        request.payload.clear();
        assert!(request
            .validate()
            .expect_err("unknown version")
            .contains("version"));
        request.version = 2;
        request.name = "host.exec".to_owned();
        assert!(request
            .validate()
            .expect_err("same closed intent")
            .contains("unsupported"));
    }

    #[test]
    fn intent_schema_is_versioned_named_and_bounded() {
        let valid = ViewIntentRequest {
            version: VIEW_INTENT_VERSION,
            name: APPLICATION_INVOKE_INTENT.to_owned(),
            payload: "hello".to_owned(),
        };
        assert!(valid.validate().is_ok());

        let mut invalid = valid.clone();
        invalid.version = VIEW_INTENT_VERSION_2 + 1;
        assert!(invalid.validate().expect_err("version").contains("version"));
        invalid = valid.clone();
        invalid.name = "shell.execute".to_owned();
        assert!(invalid
            .validate()
            .expect_err("name")
            .contains("unsupported"));
        invalid = valid;
        invalid.payload = "x".repeat(MAX_INTENT_PAYLOAD_BYTES + 1);
        assert!(invalid.validate().expect_err("payload").contains("maximum"));
    }
}
