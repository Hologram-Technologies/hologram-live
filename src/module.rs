use crate::app::AppState;
use crate::error::{LiveError, Result};
use crate::protocol::{ModuleInfo, OperationInfo, OperationKind};
use axum::Router;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

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
        assert!(!registry.supports(crate::protocol::operation::HOLO_RUN));
    }
}
