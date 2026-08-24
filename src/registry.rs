use crate::error::Result;
use crate::protocol::ObjectMetadata;
use crate::store::ObjectStore;
use std::sync::Arc;

/// Storage-facing seam for the Kappa Registry module.
///
/// The built-in implementation keeps development entirely local. A future
/// adapter can speak to the external Kappa Registry service without changing
/// module routes, native operation IDs, or desktop clients.
pub trait RegistryProvider: Send + Sync {
    fn list_objects(&self) -> Result<Vec<ObjectMetadata>>;
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
    fn list_objects(&self) -> Result<Vec<ObjectMetadata>> {
        self.store.list(None)
    }
}
