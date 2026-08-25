use crate::actor::ActorSystem;
use crate::app::AppState;
use crate::error::{LiveError, Result};
use crate::protocol::{ModuleInfo, OperationInfo, OperationKind};
use axum::Router;
use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use utoipa::openapi::{OpenApi, OpenApiBuilder};

pub type ModuleStartFuture<'a> = Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;

/// Narrow process-local services available while a module starts.
///
/// A stateful module may spawn and link Kameo actors here. Request-driven
/// modules should keep the default no-op lifecycle and avoid actor overhead.
#[derive(Clone)]
pub struct ModuleContext {
    actors: ActorSystem,
    data_dir: PathBuf,
}

impl ModuleContext {
    pub(crate) fn new(actors: ActorSystem, data_dir: PathBuf) -> Self {
        Self { actors, data_dir }
    }

    pub fn actors(&self) -> &ActorSystem {
        &self.actors
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }
}

#[derive(Debug, Clone, Copy)]
pub struct OperationDescriptor {
    pub id: &'static str,
    pub kind: OperationKind,
    pub fallback_safe_before_dispatch: bool,
}

#[derive(Debug)]
pub struct ModuleDescriptor {
    pub id: &'static str,
    pub name: &'static str,
    pub version: &'static str,
    pub dependencies: &'static [&'static str],
    pub operations: &'static [OperationDescriptor],
}

pub trait LiveModule: Send + Sync {
    fn descriptor(&self) -> &'static ModuleDescriptor;
    fn router(&self) -> Router<AppState>;

    fn openapi(&self) -> OpenApi {
        OpenApiBuilder::new().build()
    }

    fn start<'a>(&'a self, _context: &'a ModuleContext) -> ModuleStartFuture<'a> {
        Box::pin(async { Ok(()) })
    }
}

pub struct ModuleRegistry {
    modules: Vec<Arc<dyn LiveModule>>,
    operations: BTreeMap<&'static str, OperationDescriptor>,
}

impl ModuleRegistry {
    pub fn build(enabled: &[String]) -> Result<Self> {
        let available_modules = crate::modules::builtins();
        let available: BTreeMap<&str, Arc<dyn LiveModule>> = available_modules
            .into_iter()
            .map(|module| (module.descriptor().id, module))
            .collect();
        let enabled: BTreeSet<&str> = enabled.iter().map(String::as_str).collect();
        let mut visiting = BTreeSet::new();
        let mut visited = BTreeSet::new();
        let mut ordered = Vec::new();

        for id in &enabled {
            visit(
                id,
                &enabled,
                &available,
                &mut visiting,
                &mut visited,
                &mut ordered,
            )?;
        }

        let mut operations = BTreeMap::new();
        for module in &ordered {
            for operation in module.descriptor().operations {
                if operations.insert(operation.id, *operation).is_some() {
                    return Err(LiveError::Conflict(format!(
                        "operation {} is provided by more than one module",
                        operation.id
                    )));
                }
            }
        }
        Ok(Self {
            modules: ordered,
            operations,
        })
    }

    pub fn supports(&self, operation: &str) -> bool {
        self.operations.contains_key(operation)
    }

    pub fn operation(&self, operation: &str) -> Option<OperationDescriptor> {
        self.operations.get(operation).copied()
    }

    pub fn operations(&self) -> Vec<OperationInfo> {
        self.operations
            .values()
            .map(|operation| OperationInfo {
                id: operation.id.to_owned(),
                kind: operation.kind,
                fallback_safe_before_dispatch: operation.fallback_safe_before_dispatch,
            })
            .collect()
    }

    pub fn info(&self) -> Vec<ModuleInfo> {
        self.modules
            .iter()
            .map(|module| {
                let descriptor = module.descriptor();
                ModuleInfo {
                    id: descriptor.id.to_owned(),
                    name: descriptor.name.to_owned(),
                    version: descriptor.version.to_owned(),
                    state: "ready".to_owned(),
                    dependencies: descriptor
                        .dependencies
                        .iter()
                        .map(|value| (*value).to_owned())
                        .collect(),
                    operations: descriptor
                        .operations
                        .iter()
                        .map(|value| value.id.to_owned())
                        .collect(),
                }
            })
            .collect()
    }

    pub fn router(&self) -> Router<AppState> {
        self.modules.iter().fold(Router::new(), |router, module| {
            router.merge(module.router())
        })
    }

    pub async fn start(&self, context: &ModuleContext) -> Result<()> {
        for module in &self.modules {
            module.start(context).await.map_err(|error| {
                LiveError::Conflict(format!("start module {}: {error}", module.descriptor().id))
            })?;
        }
        Ok(())
    }

    pub fn openapi(&self) -> OpenApi {
        let mut document = OpenApiBuilder::new().build();
        for module in &self.modules {
            document.merge(module.openapi());
        }
        document
    }
}

fn visit(
    id: &str,
    enabled: &BTreeSet<&str>,
    available: &BTreeMap<&str, Arc<dyn LiveModule>>,
    visiting: &mut BTreeSet<String>,
    visited: &mut BTreeSet<String>,
    ordered: &mut Vec<Arc<dyn LiveModule>>,
) -> Result<()> {
    if visited.contains(id) {
        return Ok(());
    }
    if !visiting.insert(id.to_owned()) {
        return Err(LiveError::Conflict(format!(
            "module dependency cycle includes {id}"
        )));
    }
    let module = available
        .get(id)
        .ok_or_else(|| LiveError::Capability(format!("unknown module {id}")))?;
    for dependency in module.descriptor().dependencies {
        let dependency = *dependency;
        if !enabled.contains(dependency) {
            return Err(LiveError::Capability(format!(
                "module {id} requires disabled module {dependency}"
            )));
        }
        visit(dependency, enabled, available, visiting, visited, ordered)?;
    }
    visiting.remove(id);
    visited.insert(id.to_owned());
    ordered.push(module.clone());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_modules_resolve_in_dependency_order() {
        let config = crate::config::ModulesConfig::default();
        let registry = ModuleRegistry::build(&config.enabled).expect("resolve");
        assert!(registry.supports(crate::protocol::operation::HOLO_INSPECT));
        assert!(registry.supports(crate::protocol::operation::HOLO_PLAN));
        assert!(registry.supports(crate::protocol::operation::HOLO_RUN));
    }

    #[test]
    fn enabled_modules_contribute_their_openapi_paths() {
        let config = crate::config::ModulesConfig::default();
        let registry = ModuleRegistry::build(&config.enabled).expect("resolve");
        let document = registry.openapi();
        assert!(document.paths.paths.contains_key("/api/v1/modules"));
        assert!(document.paths.paths.contains_key("/api/v1/files"));
        assert!(document.paths.paths.contains_key("/api/v1/files/{id}"));
        assert!(document.paths.paths.contains_key("/api/v1/objects"));
        assert!(document.paths.paths.contains_key("/api/v1/objects/{id}"));
        assert!(document.paths.paths.contains_key("/api/v1/holo/{kappa}"));
        assert!(document
            .paths
            .paths
            .contains_key("/api/v1/holo/{kappa}/plan"));
        assert!(document
            .paths
            .paths
            .contains_key("/api/v1/holo/{kappa}/load"));
        assert!(document
            .paths
            .paths
            .contains_key("/api/v1/holo/{kappa}/run"));
        assert!(document.paths.paths.contains_key("/api/v1/holo/resident"));
        assert!(document.paths.paths.contains_key("/api/v1/chat/{id}"));
        assert!(document.paths.paths.contains_key("/api/v1/models"));
        assert!(document.paths.paths.contains_key("/api/v1/history/{id}"));
        assert!(document
            .paths
            .paths
            .contains_key("/api/v1/history/{id}/messages"));
    }
}
