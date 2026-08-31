//! Host-neutral attachment boundary for portable Hologram View layers.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, RwLock};

pub const PORTABLE_SURFACE: &str = "portable";

pub type SurfaceFuture<'a> = Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>>;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ViewAttachmentId {
    pub token: String,
    pub application_kappa: String,
    pub layer_position: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PortableViewAsset {
    pub path: String,
    pub bytes: Arc<[u8]>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PortableViewAttachment {
    pub id: ViewAttachmentId,
    pub entry: String,
    pub assets: Vec<PortableViewAsset>,
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
}
