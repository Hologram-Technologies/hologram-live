//! Locked pure-Python compilation for the import-free Component v1 world.
//!
//! `componentize-py --stub-wasi` bundles `CPython` and replaces its WASI imports
//! inside the guest. The resulting bytes are therefore an ordinary
//! `WasmCodemodule` selected by `hologram:guest/component@1`; the runtime does
//! not link ambient WASI on Python's behalf. Third-party packages are admitted
//! only when `uv.lock` pins an HTTPS, SHA-256-addressed, platform-independent
//! wheel. The exact wheels are installed into a private component build path;
//! the developer environment is never searched.

use crate::error::{LiveError, Result};
use crate::holo_python::{self, PythonProfile, PythonRootfsSource, SourceInputs};
use std::collections::{HashSet, VecDeque};
use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use std::process::Command;

pub const COMPONENTIZE_PY_VERSION: &str = "0.25.0";
pub const COMPONENT_PYTHON_VERSION: &str = "3.14";
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
    dependency_plan(&inputs).map(|_| ())
}

pub fn compile(root: &Path, source: &PythonRootfsSource) -> Result<Vec<u8>> {
    validate_source(source)?;
    let inputs = holo_python::resolve_inputs(root, source)?;
    let dependencies = dependency_plan(&inputs)?;

    let staging = tempfile::tempdir().map_err(LiveError::from)?;
    let wit = staging.path().join("hologram-application-v1.wit");
    let adapter = staging.path().join(format!("{ADAPTER_MODULE}.py"));
    let site_packages = staging.path().join("site-packages");
    let output = staging.path().join("application.component.wasm");
    fs::write(&wit, APPLICATION_WIT).map_err(|error| LiveError::io(&wit, error))?;
    fs::write(&adapter, adapter_source(&source.entry))
        .map_err(|error| LiveError::io(&adapter, error))?;
    if !dependencies.is_empty() {
        install_dependencies(staging.path(), &site_packages, &dependencies)?;
    }

    let tool = format!("componentize-py=={COMPONENTIZE_PY_VERSION}");
    let mut command = Command::new("uvx");
    command
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
        .arg(&inputs.source_dir);
    if !dependencies.is_empty() {
        command.arg("--python-path").arg(&site_packages);
    }
    command
        .arg(ADAPTER_MODULE)
        .arg("--output")
        .arg(&output)
        .current_dir(staging.path());
    isolate_python_environment(&mut command);
    let result = command.output().map_err(|error| {
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

#[derive(Clone, Debug, Eq, PartialEq)]
struct PortableDependency {
    name: String,
    version: String,
    wheel_url: String,
    sha256: String,
}

fn dependency_plan(inputs: &SourceInputs) -> Result<Vec<PortableDependency>> {
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
    if lock.get("version").and_then(toml::Value::as_integer) != Some(1) {
        return Err(LiveError::Config(format!(
            "Python lock file {} must use supported uv.lock format version 1",
            inputs.lock.display()
        )));
    }
    if let Some(revision) = lock.get("revision").and_then(toml::Value::as_integer) {
        if !(1..=3).contains(&revision) {
            return Err(LiveError::Config(format!(
                "Python lock file {} uses unsupported uv.lock revision {revision}; supported revisions are 1 through 3",
                inputs.lock.display()
            )));
        }
    }
    let packages = lock
        .get("package")
        .and_then(toml::Value::as_array)
        .ok_or_else(|| {
            LiveError::Config(format!(
                "Python lock file {} has no package records",
                inputs.lock.display()
            ))
        })?;
    let projects = packages
        .iter()
        .enumerate()
        .filter_map(|(index, package)| {
            (package
                .get("source")
                .and_then(|source| source.get("editable"))
                .and_then(toml::Value::as_str)
                == Some("."))
            .then_some(index)
        })
        .collect::<Vec<_>>();
    let [project_index] = projects.as_slice() else {
        return Err(LiveError::Config(format!(
            "Python lock file {} must contain exactly one editable project",
            inputs.lock.display()
        )));
    };

    let mut queue = package_dependencies(&packages[*project_index], &inputs.lock)?;
    let mut visited = HashSet::new();
    while let Some(reference) = queue.pop_front() {
        let index = resolve_dependency(packages, reference, *project_index, &inputs.lock)?;
        if visited.insert(index) {
            queue.extend(package_dependencies(&packages[index], &inputs.lock)?);
        }
    }
    let mut dependencies = visited
        .into_iter()
        .map(|index| portable_dependency(&packages[index], &inputs.lock))
        .collect::<Result<Vec<_>>>()?;
    dependencies.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(dependencies)
}

fn package_dependencies<'a>(
    package: &'a toml::Value,
    lock_path: &Path,
) -> Result<VecDeque<&'a toml::Value>> {
    let Some(value) = package.get("dependencies") else {
        return Ok(VecDeque::new());
    };
    let dependencies = value
        .as_array()
        .ok_or_else(|| malformed_package(lock_path, "package dependencies must be an array"))?;
    if dependencies.iter().any(|dependency| {
        dependency
            .get("name")
            .and_then(toml::Value::as_str)
            .is_none()
    }) {
        return Err(malformed_package(
            lock_path,
            "dependency reference has no name",
        ));
    }
    Ok(dependencies.iter().collect())
}

fn resolve_dependency(
    packages: &[toml::Value],
    reference: &toml::Value,
    project_index: usize,
    lock_path: &Path,
) -> Result<usize> {
    let name = reference
        .get("name")
        .and_then(toml::Value::as_str)
        .expect("package_dependencies validates names");
    let version = reference.get("version");
    let source = reference.get("source");
    let matches = packages
        .iter()
        .enumerate()
        .filter_map(|(index, package)| {
            (index != project_index
                && package.get("name").and_then(toml::Value::as_str) == Some(name)
                && version.is_none_or(|value| package.get("version") == Some(value))
                && source.is_none_or(|value| package.get("source") == Some(value)))
            .then_some(index)
        })
        .collect::<Vec<_>>();
    let [index] = matches.as_slice() else {
        let detail = if matches.is_empty() {
            "does not resolve to a package record"
        } else {
            "resolves ambiguously; lock forks must identify version and source"
        };
        return Err(LiveError::Config(format!(
            "Python lock file {} dependency {name} {detail}",
            lock_path.display()
        )));
    };
    Ok(*index)
}

fn portable_dependency(package: &toml::Value, lock_path: &Path) -> Result<PortableDependency> {
    let name = package
        .get("name")
        .and_then(toml::Value::as_str)
        .ok_or_else(|| malformed_package(lock_path, "package has no name"))?;
    let version = package
        .get("version")
        .and_then(toml::Value::as_str)
        .ok_or_else(|| malformed_package(lock_path, &format!("package {name} has no version")))?;
    if !valid_package_name(name) || version.is_empty() || version.chars().any(char::is_whitespace) {
        return Err(malformed_package(
            lock_path,
            &format!("package {name} has an unsafe name or version"),
        ));
    }
    let registry = package
        .get("source")
        .and_then(|source| source.get("registry"))
        .and_then(toml::Value::as_str);
    if registry.is_none() {
        return Err(unsupported_dependency(
            name,
            "is not pinned to a package registry",
        ));
    }
    let wheels = package
        .get("wheels")
        .and_then(toml::Value::as_array)
        .ok_or_else(|| unsupported_dependency(name, "has no locked wheel artifacts"))?;
    let mut candidates = Vec::new();
    for wheel in wheels {
        let Some(url) = wheel.get("url").and_then(toml::Value::as_str) else {
            continue;
        };
        let filename = url
            .split('?')
            .next()
            .and_then(|value| value.rsplit('/').next())
            .unwrap_or_default();
        let Some(rank) = portable_wheel_rank(filename) else {
            continue;
        };
        let Some(hash) = wheel.get("hash").and_then(toml::Value::as_str) else {
            continue;
        };
        let Some(sha256) = hash.strip_prefix("sha256:") else {
            continue;
        };
        if !url.starts_with("https://")
            || url.chars().any(char::is_whitespace)
            || sha256.len() != 64
            || !sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            continue;
        }
        candidates.push((rank, url, sha256));
    }
    candidates.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(right.1)));
    let Some((_, wheel_url, sha256)) = candidates.first() else {
        return Err(unsupported_dependency(
            name,
            "has no HTTPS, SHA-256-pinned Python 3 platform-independent wheel (*-none-any.whl)",
        ));
    };
    Ok(PortableDependency {
        name: name.to_owned(),
        version: version.to_owned(),
        wheel_url: (*wheel_url).to_owned(),
        sha256: (*sha256).to_owned(),
    })
}

fn valid_package_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        && name
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        && name
            .as_bytes()
            .last()
            .is_some_and(u8::is_ascii_alphanumeric)
}

fn portable_wheel_rank(filename: &str) -> Option<u8> {
    let stem = filename.strip_suffix(".whl")?;
    let mut tags = stem.rsplitn(4, '-');
    let platform = tags.next()?;
    let abi = tags.next()?;
    let python = tags.next()?;
    if platform != "any" || abi != "none" {
        return None;
    }
    if python.split('.').any(|tag| tag == "py3") {
        Some(if python == "py3" { 2 } else { 1 })
    } else {
        None
    }
}

fn malformed_package(lock_path: &Path, detail: &str) -> LiveError {
    LiveError::Config(format!(
        "Python lock file {} contains an invalid package record: {detail}",
        lock_path.display()
    ))
}

fn unsupported_dependency(name: &str, detail: &str) -> LiveError {
    LiveError::Capability(format!(
        "Python wasi-component dependency {name} {detail}. Use profile rootfs for native or source-only dependencies"
    ))
}

fn install_dependencies(
    staging: &Path,
    site_packages: &Path,
    dependencies: &[PortableDependency],
) -> Result<()> {
    fs::create_dir_all(site_packages).map_err(|error| LiveError::io(site_packages, error))?;
    let requirements = staging.join("locked-requirements.txt");
    let mut contents = String::new();
    for dependency in dependencies {
        writeln!(
            contents,
            "# {}=={}\n{} @ {}#sha256={}",
            dependency.name,
            dependency.version,
            dependency.name,
            dependency.wheel_url,
            dependency.sha256
        )
        .expect("writing requirements to a String cannot fail");
    }
    fs::write(&requirements, contents).map_err(|error| LiveError::io(&requirements, error))?;

    let mut command = Command::new("uv");
    command
        .args([
            "--no-config",
            "pip",
            "install",
            "--quiet",
            "--no-index",
            "--no-deps",
            "--require-hashes",
            "--only-binary",
            ":all:",
            "--python-version",
            COMPONENT_PYTHON_VERSION,
            "--link-mode",
            "copy",
            "--target",
        ])
        .arg(site_packages)
        .arg("--requirements")
        .arg(&requirements)
        .current_dir(staging);
    isolate_python_environment(&mut command);
    let result = command.output().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            LiveError::Capability(format!(
                "Python wasi-component dependency installation requires uv: {error}"
            ))
        } else {
            LiveError::Io(format!("start uv dependency installation: {error}"))
        }
    })?;
    if !result.status.success() {
        return Err(LiveError::Config(format!(
            "install locked Python component dependencies: {}",
            diagnostic(&result.stderr)
        )));
    }
    validate_installed_tree(site_packages, site_packages)
}

fn validate_installed_tree(root: &Path, directory: &Path) -> Result<()> {
    let entries = fs::read_dir(directory).map_err(|error| LiveError::io(directory, error))?;
    for entry in entries {
        let entry = entry.map_err(|error| LiveError::io(directory, error))?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|error| LiveError::io(&path, error))?;
        if metadata.file_type().is_symlink() {
            return Err(LiveError::Capability(format!(
                "Python wasi-component dependency installation produced a symlink at {}; portable dependency trees may contain regular files and directories only",
                relative_display(root, &path)
            )));
        }
        if metadata.is_dir() {
            validate_installed_tree(root, &path)?;
        } else if metadata.is_file() && is_native_payload(&path) {
            return Err(LiveError::Capability(format!(
                "Python wasi-component dependency installation produced native payload {}; use profile rootfs for native dependencies",
                relative_display(root, &path)
            )));
        }
    }
    Ok(())
}

fn is_native_payload(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "so" | "pyd" | "dylib" | "dll" | "a" | "lib"
            )
        })
}

fn relative_display(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

fn isolate_python_environment(command: &mut Command) {
    command
        .env_remove("VIRTUAL_ENV")
        .env_remove("PYTHONPATH")
        .env_remove("PYTHONHOME")
        .env_remove("PIP_CONFIG_FILE")
        .env_remove("PIP_INDEX_URL")
        .env_remove("PIP_EXTRA_INDEX_URL")
        .env_remove("UV_CONFIG_FILE")
        .env_remove("UV_INDEX")
        .env_remove("UV_DEFAULT_INDEX")
        .env_remove("UV_EXTRA_INDEX_URL")
        .env_remove("UV_FIND_LINKS")
        .env("PYTHONHASHSEED", "0")
        .env("PYTHONNOUSERSITE", "1");
    command.env("SOURCE_DATE_EPOCH", "0");
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
    fn unknown_lock_revision_is_rejected() {
        let root = project(
            "version = 1\nrevision = 99\n[[package]]\nname='demo'\nversion='0.1.0'\nsource={editable='.'}\n",
        );
        let error = check_source(root.path(), &source()).expect_err("revision rejected");
        assert_eq!(error.code(), "LIVE_CONFIG_INVALID");
        assert!(error.to_string().contains("revision 99"), "{error}");
    }

    #[test]
    fn portable_dependency_lock_is_accepted() {
        let root = project(
            "version = 1\n[[package]]\nname='demo'\nversion='0.1.0'\nsource={editable='.'}\ndependencies=[{name='six'}]\n[[package]]\nname='six'\nversion='1.17.0'\nsource={registry='https://pypi.org/simple'}\nwheels=[{url='https://files.pythonhosted.org/six-1.17.0-py2.py3-none-any.whl',hash='sha256:0000000000000000000000000000000000000000000000000000000000000000'}]\n",
        );
        check_source(root.path(), &source()).expect("portable dependency");
    }

    #[test]
    fn native_dependency_lock_is_rejected_before_componentization() {
        let root = project(
            "version = 1\n[[package]]\nname='demo'\nversion='0.1.0'\nsource={editable='.'}\ndependencies=[{name='numpy'}]\n[[package]]\nname='numpy'\nversion='2.0.0'\nsource={registry='https://pypi.org/simple'}\nwheels=[{url='https://files.pythonhosted.org/numpy-2.0.0-cp314-cp314-macosx_14_0_arm64.whl',hash='sha256:0000000000000000000000000000000000000000000000000000000000000000'}]\n",
        );
        let error = check_source(root.path(), &source()).expect_err("native dependency rejected");
        assert_eq!(error.code(), "LIVE_CAPABILITY_MISSING");
        assert!(error.to_string().contains("numpy"), "{error}");
        assert!(error.to_string().contains("none-any"), "{error}");
        assert!(error.to_string().contains("profile rootfs"), "{error}");
    }

    #[test]
    fn non_registry_dependency_is_rejected_before_componentization() {
        let root = project(
            "version = 1\n[[package]]\nname='demo'\nversion='0.1.0'\nsource={editable='.'}\ndependencies=[{name='helper'}]\n[[package]]\nname='helper'\nversion='1.0.0'\nsource={git='https://example.invalid/helper'}\nwheels=[{url='https://example.invalid/helper-1.0.0-py3-none-any.whl',hash='sha256:0000000000000000000000000000000000000000000000000000000000000000'}]\n",
        );
        let error = check_source(root.path(), &source()).expect_err("git dependency rejected");
        assert_eq!(error.code(), "LIVE_CAPABILITY_MISSING");
        assert!(error.to_string().contains("helper"), "{error}");
        assert!(error.to_string().contains("package registry"), "{error}");
    }

    #[test]
    fn unsafe_dependency_name_is_rejected_before_requirements_generation() {
        let root = project(
            "version = 1\n[[package]]\nname='demo'\nversion='0.1.0'\nsource={editable='.'}\ndependencies=[{name='six\\n--extra-index-url'}]\n[[package]]\nname='six\\n--extra-index-url'\nversion='1.17.0'\nsource={registry='https://pypi.org/simple'}\nwheels=[{url='https://files.pythonhosted.org/six-1.17.0-py3-none-any.whl',hash='sha256:0000000000000000000000000000000000000000000000000000000000000000'}]\n",
        );
        let error = check_source(root.path(), &source()).expect_err("unsafe package rejected");
        assert_eq!(error.code(), "LIVE_CONFIG_INVALID");
        assert!(
            error.to_string().contains("unsafe name or version"),
            "{error}"
        );
    }

    #[test]
    fn unreferenced_development_package_is_not_componentized() {
        let root = project(
            "version = 1\n[[package]]\nname='demo'\nversion='0.1.0'\nsource={editable='.'}\n[package.dev-dependencies]\nnative=[{name='numpy'}]\n[[package]]\nname='numpy'\nversion='2.0.0'\nsource={registry='https://pypi.org/simple'}\nwheels=[{url='https://files.pythonhosted.org/numpy-2.0.0-cp314-cp314-macosx_14_0_arm64.whl',hash='sha256:0000000000000000000000000000000000000000000000000000000000000000'}]\n",
        );
        check_source(root.path(), &source()).expect("development dependency ignored");
    }

    #[test]
    fn installed_native_payload_is_rejected() {
        let root = tempfile::tempdir().expect("dependency tree");
        let native = root.path().join("package/native.so");
        fs::create_dir_all(native.parent().expect("native parent")).expect("native parent");
        fs::write(&native, b"not native, but named like it").expect("native fixture");
        let error =
            validate_installed_tree(root.path(), root.path()).expect_err("native payload rejected");
        assert_eq!(error.code(), "LIVE_CAPABILITY_MISSING");
        assert!(error.to_string().contains("package/native.so"), "{error}");
    }

    #[test]
    fn adapter_calls_the_declared_bytes_entrypoint() {
        let adapter = adapter_source("package.worker:main");
        assert!(adapter.contains("import_module(\"package.worker\")"));
        assert!(adapter.contains("\"main\""));
        assert!(adapter.contains("must return bytes"));
    }
}
