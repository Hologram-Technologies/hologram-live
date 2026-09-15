//! The built-in provider. Content lives in the local `ObjectStore`, so this
//! path needs no network and no external service.

use super::RegistryProvider;
use crate::error::Result;
use crate::protocol::{ObjectContent, ObjectMetadata};
use crate::store::ObjectStore;
use std::sync::Arc;

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
