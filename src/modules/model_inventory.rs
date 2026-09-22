//! Unified model inventory for compatibility APIs.

use crate::error::{LiveError, Result};
use crate::inference::InferenceEngine;
use crate::models::{ModelCatalog, ModelInfo};
use std::collections::HashSet;
use std::sync::Arc;

/// Returns imported models followed by models advertised by the active
/// engine. An engine such as weightc or llama.cpp may report an imported model
/// again, so IDs are de-duplicated while preserving the catalog entry.
pub(crate) async fn list(
    catalog: Arc<ModelCatalog>,
    engine: Arc<dyn InferenceEngine>,
) -> Result<Vec<ModelInfo>> {
    let models = tokio::task::spawn_blocking(move || catalog.list())
        .await
        .map_err(|error| LiveError::Conflict(format!("join model listing: {error}")))??;
    Ok(merge(models, engine.list_models().await?))
}

fn merge(mut models: Vec<ModelInfo>, engine_models: Vec<ModelInfo>) -> Vec<ModelInfo> {
    let mut seen: HashSet<String> = models.iter().map(|model| model.id.clone()).collect();
    for model in engine_models {
        if seen.insert(model.id.clone()) {
            models.push(model);
        }
    }
    models
}

/// Resolves an imported model first, then asks the active engine. This keeps
/// `/api/show` useful for remote Ollama and vLLM models as well as local
/// catalog artifacts.
pub(crate) async fn find(
    catalog: Arc<ModelCatalog>,
    engine: Arc<dyn InferenceEngine>,
    name: &str,
) -> Result<ModelInfo> {
    let lookup = name.to_owned();
    let catalog_lookup = lookup.clone();
    let imported = tokio::task::spawn_blocking(move || catalog.resolve(&catalog_lookup))
        .await
        .map_err(|error| LiveError::Conflict(format!("join model lookup: {error}")))?;
    match imported {
        Ok(model) => return Ok(model),
        Err(LiveError::NotFound(_)) => {}
        Err(error) => return Err(error),
    }
    engine
        .list_models()
        .await?
        .into_iter()
        .find(|model| model.id == lookup || model.name == lookup)
        .ok_or_else(|| LiveError::NotFound(format!("model {name:?} not found")))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct RemoteEngine;

    #[tonic::async_trait]
    impl InferenceEngine for RemoteEngine {
        fn name(&self) -> &'static str {
            "remote"
        }

        async fn complete(
            &self,
            _request: crate::inference::CompletionRequest,
        ) -> Result<crate::inference::Completion> {
            unreachable!("inventory tests do not complete prompts")
        }

        async fn list_models(&self) -> Result<Vec<ModelInfo>> {
            Ok(vec![model("remote", "vllm")])
        }
    }

    #[test]
    fn merge_policy_preserves_catalog_entry_and_adds_remote_models() {
        let local = vec![model("shared", "weightc")];
        let merged = merge(
            local,
            vec![model("shared", "vllm"), model("remote", "vllm")],
        );

        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].engine, "weightc");
        assert_eq!(merged[1].id, "remote");
    }

    #[tokio::test]
    async fn inventory_lists_and_finds_engine_models() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let store = Arc::new(
            crate::store::ObjectStore::open(temporary.path().join("store")).expect("store"),
        );
        let catalog =
            Arc::new(ModelCatalog::open(store, temporary.path().join("models")).expect("catalog"));
        let engine: Arc<dyn InferenceEngine> = Arc::new(RemoteEngine);

        let models = list(catalog.clone(), engine.clone())
            .await
            .expect("inventory");
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "remote");

        let found = find(catalog, engine, "remote").await.expect("find remote");
        assert_eq!(found.engine, "vllm");
    }

    fn model(id: &str, engine: &str) -> ModelInfo {
        ModelInfo {
            id: id.to_owned(),
            name: id.to_owned(),
            engine: engine.to_owned(),
            source: String::new(),
            size: 0,
            created_at_millis: 0,
        }
    }
}
