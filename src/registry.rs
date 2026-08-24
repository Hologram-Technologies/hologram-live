use crate::error::Result;
use crate::protocol::{ObjectContent, ObjectMetadata};
use crate::store::ObjectStore;
use std::sync::Arc;

/// Storage-facing seam for the Kappa Registry module.
///
/// The built-in implementation keeps development entirely local. A future
/// adapter can speak to the external Kappa Registry service without changing
/// module routes, native operation IDs, or desktop clients.
pub trait RegistryProvider: Send + Sync {
    fn list_objects(&self, kind: Option<&str>) -> Result<Vec<ObjectMetadata>>;
    fn put_object(
        &self,
        kind: String,
        media_type: String,
        filename: Option<String>,
        bytes: &[u8],
    ) -> Result<ObjectMetadata>;
    fn get_object(&self, id: &str) -> Result<ObjectContent>;
    fn rename_file(&self, id: &str, filename: String) -> Result<ObjectMetadata>;
}

pub struct LocalRegistryProvider {
    store: Arc<ObjectStore>,
}

impl LocalRegistryProvider {
    pub fn new(store: Arc<ObjectStore>) -> Self {
        Self { store }
    }
}

impl RegistryProvider for LocalRegistryProvider {
    fn list_objects(&self, kind: Option<&str>) -> Result<Vec<ObjectMetadata>> {
        self.store.list(kind)
    }

    fn put_object(
        &self,
        kind: String,
        media_type: String,
        filename: Option<String>,
        bytes: &[u8],
    ) -> Result<ObjectMetadata> {
        self.store.put(kind, media_type, filename, bytes)
    }

    fn get_object(&self, id: &str) -> Result<ObjectContent> {
        Ok(ObjectContent {
            metadata: self.store.metadata(id)?,
            bytes: self.store.get(id)?,
        })
    }

    fn rename_file(&self, id: &str, filename: String) -> Result<ObjectMetadata> {
        self.store.rename_file(id, filename)
    }
}
