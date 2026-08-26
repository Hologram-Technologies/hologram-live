//! Dependency-free Python compilation for the import-free Component v1 world.
//!
//! `componentize-py --stub-wasi` bundles `CPython` and replaces its WASI imports
//! inside the guest. The resulting bytes are therefore an ordinary
//! `WasmCodemodule` selected by `hologram:guest/component@1`; the runtime does
//! not link ambient WASI on Python's behalf.

use crate::error::{LiveError, Result};
use crate::holo_python::{self, PythonProfile, PythonRootfsSource, SourceInputs};
use std::fs;
use std::path::Path;
use std::process::Command;

pub const COMPONENTIZE_PY_VERSION: &str = "0.25.0";
const ADAPTER_MODULE: &str = "_hologram_guest";
const DEFAULT_ROOTFS_BASE: &str = "python:3.12-slim";
const APPLICATION_WIT: &str = include_str!("../specs/wit/hologram-application-v1.wit");

pub fn validate_source(source: &PythonRootfsSource) -> Result<()> {
    if source.profile != PythonProfile::WasiComponent {
        return Err(LiveError::Config(
            "Python Component compilation requires profile wasi-component".to_owned(),
        ));
    }
    holo_python::validate_entry(&source.entry)?;
    if source.project.is_absolute() || source.lock.is_absolute() {
        return Err(LiveError::Config(
            "Python project and lock paths must be relative to hologram.json".to_owned(),
        ));
    }
    if source.base != DEFAULT_ROOTFS_BASE {
        return Err(LiveError::Config(
            "Python wasi-component sources do not accept an OCI base image".to_owned(),
        ));
    }
    Ok(())
}

pub fn check_source(root: &Path, source: &PythonRootfsSource) -> Result<()> {
    validate_source(source)?;
    let inputs = holo_python::resolve_inputs(root, source)?;
    validate_dependency_free_lock(&inputs)
}

pub fn compile(root: &Path, source: &PythonRootfsSource) -> Result<Vec<u8>> {
    validate_source(source)?;
    let inputs = holo_python::resolve_inputs(root, source)?;
    validate_dependency_free_lock(&inputs)?;

    let staging = tempfile::tempdir().map_err(LiveError::from)?;
    let wit = staging.path().join("hologram-application-v1.wit");
    let adapter = staging.path().join(format!("{ADAPTER_MODULE}.py"));
    let output = staging.path().join("application.component.wasm");
    fs::write(&wit, APPLICATION_WIT).map_err(|error| LiveError::io(&wit, error))?;
    fs::write(&adapter, adapter_source(&source.entry))
        .map_err(|error| LiveError::io(&adapter, error))?;

    let tool = format!("componentize-py=={COMPONENTIZE_PY_VERSION}");
    let result = Command::new("uvx")
        .args([
            "--isolated",
            "--no-config",
            "--from",
            &tool,
            "componentize-py",
            "--quiet",
            "--wit-path",
        ])
        .arg(&wit)
        .args([
            "--world",
            "application",
            "componentize",
            "--stub-wasi",
            "--python-path",
        ])
        .arg(staging.path())
        .arg("--python-path")
        .arg(&inputs.source_dir)
        .arg(ADAPTER_MODULE)
        .arg("--output")
        .arg(&output)
        .current_dir(staging.path())
        .env_remove("VIRTUAL_ENV")
        .env_remove("PYTHONPATH")
        .env_remove("PYTHONHOME")
        .env("PYTHONNOUSERSITE", "1")
        .output()
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                LiveError::Capability(format!(
                    "Python wasi-component compilation requires uvx and pinned {tool}: {error}"
                ))
            } else {
                LiveError::Io(format!("start pinned {tool}: {error}"))
            }
        })?;
    if !result.status.success() {
        return Err(LiveError::Config(format!(
            "pinned {tool} failed: {}",
            diagnostic(&result.stderr)
        )));
    }
    fs::read(&output).map_err(|error| LiveError::io(&output, error))
}

fn validate_dependency_free_lock(inputs: &SourceInputs) -> Result<()> {
    let bytes = fs::read(&inputs.lock).map_err(|error| LiveError::io(&inputs.lock, error))?;
    let text = std::str::from_utf8(&bytes).map_err(|error| {
        LiveError::Config(format!(
            "Python lock file {} is not UTF-8: {error}",
            inputs.lock.display()
        ))
    })?;
    let lock: toml::Value = toml::from_str(text).map_err(|error| {
        LiveError::Config(format!(
            "parse Python lock file {}: {error}",
            inputs.lock.display()
        ))
    })?;
    let packages = lock
        .get("package")
        .and_then(toml::Value::as_array)
        .ok_or_else(|| {
            LiveError::Config(format!(
                "Python lock file {} has no package records",
                inputs.lock.display()
            ))
        })?;
    let mut found_project = false;
    let mut external = Vec::new();
    for package in packages {
        let editable_project = package
            .get("source")
            .and_then(|source| source.get("editable"))
            .and_then(toml::Value::as_str)
            == Some(".");
        if editable_project && !found_project {
            found_project = true;
        } else {
            external.push(
                package
                    .get("name")
                    .and_then(toml::Value::as_str)
                    .unwrap_or("<unnamed>"),
            );
        }
    }
    if !found_project {
        return Err(LiveError::Config(format!(
            "Python lock file {} does not contain the editable project",
            inputs.lock.display()
        )));
    }
    if !external.is_empty() {
        return Err(LiveError::Capability(format!(
            "Python wasi-component currently accepts dependency-free locks only; unsupported locked packages: {}. Use profile rootfs for native dependencies or wait for the locked pure-Python component profile",
            external.join(", ")
        )));
    }
    Ok(())
}

fn adapter_source(entry: &str) -> String {
    let (module, function) = entry
        .split_once(':')
        .expect("validated Python entries contain a separator");
    format!(
        "import importlib\n\nfrom wit_world import exports\n\n_entrypoint = getattr(importlib.import_module(\"{module}\"), \"{function}\")\n\n\nclass Guest(exports.Guest):\n    def run(self, input: bytes) -> bytes:\n        output = _entrypoint(input)\n        if not isinstance(output, bytes):\n            raise TypeError(\"Hologram Python entrypoint must return bytes\")\n        return output\n"
    )
}

fn diagnostic(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let trimmed = text.trim();
    if trimmed.chars().count() <= 4_096 {
        trimmed.to_owned()
    } else {
        let tail = trimmed.chars().rev().take(4_096).collect::<Vec<_>>();
        format!("…{}", tail.into_iter().rev().collect::<String>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn source() -> PythonRootfsSource {
        PythonRootfsSource {
            project: PathBuf::from("project"),
            entry: "demo:main".to_owned(),
            lock: PathBuf::from("uv.lock"),
            profile: PythonProfile::WasiComponent,
            base: DEFAULT_ROOTFS_BASE.to_owned(),
        }
    }

    fn project(lock: &str) -> tempfile::TempDir {
        let root = tempfile::tempdir().expect("root");
        let project = root.path().join("project");
        fs::create_dir_all(project.join("src/demo")).expect("source directory");
        fs::write(
            project.join("pyproject.toml"),
            "[project]\nname='demo'\nversion='0.1.0'\ndependencies=[]\n",
        )
        .expect("project metadata");
        fs::write(project.join("uv.lock"), lock).expect("lock");
        fs::write(
            project.join("src/demo/__init__.py"),
            "def main(value): return value\n",
        )
        .expect("source");
        root
    }

    #[test]
    fn dependency_free_lock_is_accepted() {
        let root = project(
            "version = 1\n[[package]]\nname='demo'\nversion='0.1.0'\nsource={editable='.'}\n",
        );
        check_source(root.path(), &source()).expect("dependency-free project");
    }

    #[test]
    fn dependency_lock_is_rejected_before_componentization() {
        let root = project(
            "version = 1\n[[package]]\nname='demo'\nversion='0.1.0'\nsource={editable='.'}\n[[package]]\nname='requests'\nversion='2.0.0'\nsource={registry='https://example.invalid'}\n",
        );
        let error = check_source(root.path(), &source()).expect_err("dependency rejected");
        assert_eq!(error.code(), "LIVE_CAPABILITY_MISSING");
        assert!(error.to_string().contains("requests"), "{error}");
    }

    #[test]
    fn adapter_calls_the_declared_bytes_entrypoint() {
        let adapter = adapter_source("package.worker:main");
        assert!(adapter.contains("import_module(\"package.worker\")"));
        assert!(adapter.contains("\"main\""));
        assert!(adapter.contains("must return bytes"));
    }
}
