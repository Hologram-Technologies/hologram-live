//! Storage-facing seam for the Kappa Registry module.
//!
//! The trait is deliberately synchronous. Every caller already runs provider
//! work inside `tokio::task::spawn_blocking` (see `src/modules/registry.rs`
//! and `src/modules/files.rs`), and an async trait would force boxed futures
//! at every call site because `async fn` in traits is not `dyn`-safe.

/// Public so the artifact-distribution pull path can reach the client
/// directly. Keeping it private would force a duplicate HTTP client there.
mod kappa;
pub mod kappa_client;
mod local;

pub use kappa::KappaRegistryProvider;
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

/// Refuse bytes that do not hash to the id they were read under.
///
/// An id is the BLAKE3 of the content, so a provider can always tell whether
/// what its backend returned is what was stored. Without this check a flipped
/// bit on disk, or in a registry's blob store, is served with a 200 and an
/// `ETag` naming the content the caller asked for. It is a storage fault,
/// not a caller's mistake, so it surfaces as `LIVE_IO`.
pub(crate) fn verify_content(id: &str, bytes: &[u8]) -> Result<()> {
    let actual = format!("blake3:{}", blake3::hash(bytes).to_hex());
    if actual == id {
        return Ok(());
    }
    Err(crate::error::LiveError::Io(format!(
        "object {id} is corrupt: the stored bytes hash to {actual}"
    )))
}

/// Build the provider named by configuration.
///
/// Local is the default so a stock install needs no external service;
/// `AppConfig::validate` has already rejected an unknown name or a kappa
/// provider with no endpoint, so this cannot fail on a validated config.
pub fn provider_from_config(
    config: &crate::config::AppConfig,
    store: std::sync::Arc<crate::store::ObjectStore>,
) -> Result<std::sync::Arc<dyn RegistryProvider>> {
    match config.registry.provider.as_str() {
        "kappa" => Ok(std::sync::Arc::new(KappaRegistryProvider::new(
            &config.registry,
        )?)),
        _ => Ok(std::sync::Arc::new(LocalRegistryProvider::new(store))),
    }
}

#[cfg(test)]
mod tests {
    use super::verify_content;

    #[test]
    fn bytes_are_accepted_only_under_their_own_address() {
        let bytes = b"what was stored";
        let id = format!("blake3:{}", blake3::hash(bytes).to_hex());
        verify_content(&id, bytes).expect("intact bytes pass");

        let error = verify_content(&id, b"what came back").expect_err("other bytes are refused");
        assert_eq!(error.code(), "LIVE_IO");
        assert!(error.to_string().contains(&id), "{error}");
    }
}
