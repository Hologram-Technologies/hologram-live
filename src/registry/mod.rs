//! Storage-facing seam for the Kappa Registry module.
//!
//! The trait is deliberately synchronous. Every caller already runs provider
//! work inside `tokio::task::spawn_blocking` (see `src/modules/registry.rs`
//! and `src/modules/files.rs`), and an async trait would force boxed futures
//! at every call site because `async fn` in traits is not `dyn`-safe.

mod local;

pub use local::LocalRegistryProvider;

use crate::error::Result;
use crate::protocol::{ObjectContent, ObjectMetadata, ObjectPage, ObjectQuery};

/// Storage-facing seam for the Kappa Registry module.
///
/// `LocalRegistryProvider` keeps development entirely local. A future adapter
/// can speak to the external Kappa Registry service without changing module
/// routes, native operation IDs, or desktop clients.
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

    /// Page through objects matching `query`, ascending by id.
    ///
    /// `list_objects` keeps its newest-first contract and bare-array response
    /// for existing callers; only this surface is id-ordered.
    fn search(&self, query: &ObjectQuery) -> Result<ObjectPage>;
}
