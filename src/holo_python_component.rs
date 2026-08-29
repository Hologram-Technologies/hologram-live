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
use crate::util::hex;
use hologram::space::address_bytes;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{HashSet, VecDeque};
use std::fmt::Write as _;
use std::fs;
use std::io::Read as _;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

pub const COMPONENTIZE_PY_VERSION: &str = "0.25.0";
pub const COMPONENT_PYTHON_VERSION: &str = "3.14.0";
const COMPONENT_PYTHON_INSTALL_VERSION: &str = "3.14";
const COMPONENTIZE_PY_SOURCE_REVISION: &str = "c0949b19d464f5d70bc1051195a3ae0e6a012df9";
const COMPONENTIZER_RELEASE_TAG: &str = "componentizer-v0.25.0-hologram.5";
const COMPONENTIZER_RELEASE_URL: &str = "https://github.com/Hologram-Technologies/hologram-live/releases/tag/componentizer-v0.25.0-hologram.5";
const COMPONENTIZER_PATCHSET_URL: &str = "https://github.com/Hologram-Technologies/hologram-live/releases/download/componentizer-v0.25.0-hologram.5/PATCHSET.sha256";
const COMPONENTIZER_PATCHSET_SHA256: &str =
    "8262cb4562428132c29dc4a46780178a5e0f4d7fa1c41549e2f15c76f7dec8ad";
const COMPONENTIZER_DETERMINISM_CONTRACT: &str =
    "hologram:componentizer/preinitialization-determinism@5";
const TARGET_ABI: &str = "wasm32-wasip2-component";
const GUEST_CONTRACT: &str = "hologram:guest/component@1";
const REPRODUCIBILITY_BLOCKER: &str = "the deterministic componentizer is pinned, but two independent clean builds have not yet been compared on every supported host";
const ADAPTER_MODULE: &str = "_hologram_guest";
const DEFAULT_ROOTFS_BASE: &str = "python:3.12-slim";
const APPLICATION_WIT: &str = include_str!("../specs/wit/hologram-application-v1.wit");

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BuildTool {
    pub name: &'static str,
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_revision: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub distribution: Option<ToolDistribution>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub patch_set: Option<ToolPatchSet>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ToolDistribution {
    pub url: &'static str,
    pub sha256: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ToolPatchSet {
    pub release_tag: &'static str,
    pub release_url: &'static str,
    pub manifest_url: &'static str,
    pub manifest_sha256: &'static str,
    pub determinism_contract: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BuildHost {
    pub os: &'static str,
    pub arch: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BuildInput {
    pub role: &'static str,
    pub path: String,
    pub sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PortableDependency {
    pub name: String,
    pub version: String,
    pub wheel_url: String,
    pub sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BuildOutput {
    pub layer_kappa: String,
    pub byte_length: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Reproducibility {
    pub reproducible: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocker: Option<&'static str>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BuildProvenance {
    pub profile: &'static str,
    pub guest_contract: &'static str,
    pub target_abi: &'static str,
    pub build_host: BuildHost,
    pub compiler: BuildTool,
    pub runtime: BuildTool,
    pub componentizer: BuildTool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub componentizer_runner: Option<BuildTool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dependency_installer: Option<BuildTool>,
    pub inputs: Vec<BuildInput>,
    pub dependencies: Vec<PortableDependency>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<BuildOutput>,
    pub reproducibility: Reproducibility,
}

pub struct CompiledComponent {
    pub bytes: Vec<u8>,
    pub provenance: BuildProvenance,
}

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

pub fn check_source(root: &Path, source: &PythonRootfsSource) -> Result<BuildProvenance> {
    validate_source(source)?;
    let inputs = holo_python::resolve_inputs(root, source)?;
    let dependencies = dependency_plan(&inputs)?;
    build_provenance(source, &inputs, dependencies)
}

pub fn compile(root: &Path, source: &PythonRootfsSource) -> Result<CompiledComponent> {
    validate_source(source)?;
    let inputs = holo_python::resolve_inputs(root, source)?;
    let dependencies = dependency_plan(&inputs)?;
    let mut provenance = build_provenance(source, &inputs, dependencies.clone())?;

    let staging = tempfile::tempdir().map_err(LiveError::from)?;
    let wit = staging.path().join("hologram-application-v1.wit");
    let adapter = staging.path().join(format!("{ADAPTER_MODULE}.py"));
    let source_dir = staging.path().join("application-source");
    let site_packages = staging.path().join("site-packages");
    let output = staging.path().join("application.component.wasm");
    fs::write(&wit, APPLICATION_WIT).map_err(|error| LiveError::io(&wit, error))?;
    fs::write(&adapter, adapter_source(&source.entry))
        .map_err(|error| LiveError::io(&adapter, error))?;
    copy_source_tree(&inputs.source_dir, &source_dir)?;
    let source_input = provenance
        .inputs
        .iter_mut()
        .find(|input| input.role == "source-tree")
        .ok_or_else(|| {
            LiveError::Conflict("Python Component provenance lost its source-tree input".to_owned())
        })?;
    source_input.sha256 = sha256_source_tree(&source_dir)?;
    if !dependencies.is_empty() {
        install_dependencies(staging.path(), &site_packages, &dependencies)?;
        provenance.dependency_installer = Some(tool_version("uv")?);
    }

    let distribution = provenance
        .componentizer
        .distribution
        .as_ref()
        .ok_or_else(|| {
            LiveError::Conflict("componentizer distribution pin is missing".to_owned())
        })?;
    let tool = format!(
        "componentize-py @ {}#sha256={}",
        distribution.url, distribution.sha256
    );
    let tool_label = format!(
        "componentize-py {COMPONENTIZE_PY_VERSION} {}/{} wheel sha256:{}",
        provenance.build_host.os, provenance.build_host.arch, distribution.sha256
    );
    let mut command = Command::new("uvx");
    command
        .args([
            "--isolated",
            "--no-config",
            "--no-index",
            "--no-build",
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
        .arg(&source_dir);
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
                "Python wasi-component compilation requires uvx and pinned {tool_label}: {error}"
            ))
        } else {
            LiveError::Io(format!("start pinned {tool_label}: {error}"))
        }
    })?;
    if !result.status.success() {
        return Err(LiveError::Config(format!(
            "pinned {tool_label} failed: {}",
            diagnostic(&result.stderr)
        )));
    }
    let bytes = fs::read(&output).map_err(|error| LiveError::io(&output, error))?;
    provenance.componentizer_runner = Some(tool_version("uvx")?);
    provenance.output = Some(BuildOutput {
        layer_kappa: address_bytes(&bytes).to_string(),
        byte_length: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
    });
    provenance.reproducibility = Reproducibility {
        reproducible: true,
        blocker: None,
    };
    Ok(CompiledComponent { bytes, provenance })
}

fn build_provenance(
    source: &PythonRootfsSource,
    inputs: &SourceInputs,
    dependencies: Vec<PortableDependency>,
) -> Result<BuildProvenance> {
    let host_os = std::env::consts::OS;
    let host_arch = std::env::consts::ARCH;
    let componentizer_distribution = componentizer_distribution(host_os, host_arch)?;
    let inputs = source_build_inputs(source, inputs)?;
    Ok(BuildProvenance {
        profile: "wasi-component",
        guest_contract: GUEST_CONTRACT,
        target_abi: TARGET_ABI,
        build_host: BuildHost {
            os: host_os,
            arch: host_arch,
        },
        compiler: BuildTool {
            name: "hologram-live",
            version: env!("CARGO_PKG_VERSION").to_owned(),
            source_revision: None,
            distribution: None,
            patch_set: None,
        },
        runtime: BuildTool {
            name: "cpython",
            version: COMPONENT_PYTHON_VERSION.to_owned(),
            source_revision: None,
            distribution: None,
            patch_set: None,
        },
        componentizer: BuildTool {
            name: "componentize-py",
            version: COMPONENTIZE_PY_VERSION.to_owned(),
            source_revision: Some(COMPONENTIZE_PY_SOURCE_REVISION),
            distribution: Some(componentizer_distribution),
            patch_set: Some(componentizer_patch_set()),
        },
        componentizer_runner: None,
        dependency_installer: None,
        inputs,
        dependencies,
        output: None,
        reproducibility: Reproducibility {
            reproducible: false,
            blocker: Some(REPRODUCIBILITY_BLOCKER),
        },
    })
}

pub(crate) fn source_build_inputs(
    source: &PythonRootfsSource,
    inputs: &SourceInputs,
) -> Result<Vec<BuildInput>> {
    let project = logical_path(&source.project)?;
    let input = |role: &'static str,
                 suffix: &Path,
                 path: &Path,
                 sha256: fn(&Path) -> Result<String>|
     -> Result<BuildInput> {
        Ok(BuildInput {
            role,
            path: logical_path(&source.project.join(suffix))?,
            sha256: sha256(path)?,
        })
    };
    Ok(vec![
        input(
            "project-metadata",
            Path::new("pyproject.toml"),
            &inputs.pyproject,
            sha256_file,
        )?,
        input("lock", &source.lock, &inputs.lock, sha256_file)?,
        BuildInput {
            role: "source-tree",
            path: if project == "." {
                "src".to_owned()
            } else {
                format!("{project}/src")
            },
            sha256: sha256_source_tree(&inputs.source_dir)?,
        },
    ])
}

fn tool_version(name: &'static str) -> Result<BuildTool> {
    let output = Command::new(name)
        .arg("--version")
        .output()
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                LiveError::Capability(format!(
                    "Python wasi-component compilation requires {name}: {error}"
                ))
            } else {
                LiveError::Io(format!("query {name} version: {error}"))
            }
        })?;
    if !output.status.success() {
        return Err(LiveError::Config(format!(
            "query {name} version: {}",
            diagnostic(&output.stderr)
        )));
    }
    let text = std::str::from_utf8(&output.stdout)
        .map_err(|error| LiveError::Config(format!("{name} version is not UTF-8: {error}")))?;
    let version = text
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| LiveError::Config(format!("unrecognized {name} version: {text:?}")))?;
    Ok(BuildTool {
        name,
        version: version.to_owned(),
        source_revision: None,
        distribution: None,
        patch_set: None,
    })
}

fn componentizer_patch_set() -> ToolPatchSet {
    ToolPatchSet {
        release_tag: COMPONENTIZER_RELEASE_TAG,
        release_url: COMPONENTIZER_RELEASE_URL,
        manifest_url: COMPONENTIZER_PATCHSET_URL,
        manifest_sha256: COMPONENTIZER_PATCHSET_SHA256,
        determinism_contract: COMPONENTIZER_DETERMINISM_CONTRACT,
    }
}

fn componentizer_distribution(os: &str, arch: &str) -> Result<ToolDistribution> {
    let distribution = match (os, arch) {
        ("macos", "x86_64") => ToolDistribution {
            url: "https://github.com/Hologram-Technologies/hologram-live/releases/download/componentizer-v0.25.0-hologram.5/componentize_py-0.25.0-cp39-abi3-macosx_10_12_x86_64.whl",
            sha256: "4653f85787ce1fd8f21abeb3ed07f940367a6a8f16df7bc7279131a0252a4da1",
        },
        ("macos", "aarch64") => ToolDistribution {
            url: "https://github.com/Hologram-Technologies/hologram-live/releases/download/componentizer-v0.25.0-hologram.5/componentize_py-0.25.0-cp39-abi3-macosx_11_0_arm64.whl",
            sha256: "eb9a6ed5c5d93ef949bcf2682b64b9097d1fa13b8f87fe3aabe54be7415559f8",
        },
        ("linux", "x86_64") => ToolDistribution {
            url: "https://github.com/Hologram-Technologies/hologram-live/releases/download/componentizer-v0.25.0-hologram.5/componentize_py-0.25.0-cp39-abi3-manylinux_2_28_x86_64.whl",
            sha256: "1285eeb7cec8408153523016228f2afe577357419101373dc94b77fe54d7973f",
        },
        ("linux", "aarch64") => ToolDistribution {
            url: "https://github.com/Hologram-Technologies/hologram-live/releases/download/componentizer-v0.25.0-hologram.5/componentize_py-0.25.0-cp39-abi3-manylinux_2_28_aarch64.whl",
            sha256: "72e6ae13ff1b597e2e7adfafb80b562463acfbedb69603e3f0b36f83c895c365",
        },
        ("windows", "x86_64") => ToolDistribution {
            url: "https://github.com/Hologram-Technologies/hologram-live/releases/download/componentizer-v0.25.0-hologram.5/componentize_py-0.25.0-cp39-abi3-win_amd64.whl",
            sha256: "fdf254a2d3ec235921a4a7a62d63b0c39e7b7da4444f08ba6cad730c35965a39",
        },
        _ => {
            return Err(LiveError::Capability(format!(
                "Python wasi-component compilation has no pinned componentize-py {COMPONENTIZE_PY_VERSION} wheel for host {os}/{arch}; supported hosts are macos/aarch64, macos/x86_64, linux/aarch64, linux/x86_64, and windows/x86_64"
            )));
        }
    };
    Ok(distribution)
}

fn logical_path(path: &Path) -> Result<String> {
    if path.as_os_str().is_empty() {
        return Ok(".".to_owned());
    }
    let mut normalized = Vec::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(value) => normalized.push(value.to_str().ok_or_else(|| {
                LiveError::Config(format!(
                    "Python source path {} is not UTF-8",
                    path.display()
                ))
            })?),
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(LiveError::Config(format!(
                    "Python source path {} is not a normalized relative path",
                    path.display()
                )));
            }
        }
    }
    Ok(if normalized.is_empty() {
        ".".to_owned()
    } else {
        normalized.join("/")
    })
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path).map_err(|error| LiveError::io(path, error))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|error| LiveError::io(path, error))?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(hex(&hasher.finalize()))
}

fn source_files(root: &Path) -> Result<Vec<(PathBuf, String)>> {
    fn visit(root: &Path, directory: &Path, files: &mut Vec<(PathBuf, String)>) -> Result<()> {
        for entry in fs::read_dir(directory).map_err(|error| LiveError::io(directory, error))? {
            let entry = entry.map_err(|error| LiveError::io(directory, error))?;
            let path = entry.path();
            let metadata =
                fs::symlink_metadata(&path).map_err(|error| LiveError::io(&path, error))?;
            if metadata.file_type().is_symlink() {
                return Err(LiveError::Config(format!(
                    "Python source tree contains a symlink at {}; source inputs must be regular files and directories",
                    path.strip_prefix(root).unwrap_or(&path).display()
                )));
            }
            if metadata.is_dir() {
                visit(root, &path, files)?;
            } else if metadata.is_file() {
                let relative = path.strip_prefix(root).map_err(|error| {
                    LiveError::Config(format!(
                        "Python source {} escapes {}: {error}",
                        path.display(),
                        root.display()
                    ))
                })?;
                files.push((path.clone(), logical_path(relative)?));
            } else {
                return Err(LiveError::Config(format!(
                    "Python source tree contains unsupported input {}",
                    path.display()
                )));
            }
        }
        Ok(())
    }

    let mut files = Vec::new();
    visit(root, root, &mut files)?;
    files.sort_by(|left, right| left.1.cmp(&right.1));
    Ok(files)
}

fn sha256_source_tree(root: &Path) -> Result<String> {
    let mut hasher = Sha256::new();
    hasher.update(b"hologram-python-source-tree-v1\0");
    for (path, relative) in source_files(root)? {
        let path_bytes = relative.as_bytes();
        hasher.update(
            u64::try_from(path_bytes.len())
                .unwrap_or(u64::MAX)
                .to_le_bytes(),
        );
        hasher.update(path_bytes);
        let metadata = fs::metadata(&path).map_err(|error| LiveError::io(&path, error))?;
        hasher.update(metadata.len().to_le_bytes());
        let mut file = fs::File::open(&path).map_err(|error| LiveError::io(&path, error))?;
        let mut buffer = vec![0_u8; 64 * 1024];
        loop {
            let count = file
                .read(&mut buffer)
                .map_err(|error| LiveError::io(&path, error))?;
            if count == 0 {
                break;
            }
            hasher.update(&buffer[..count]);
        }
    }
    Ok(hex(&hasher.finalize()))
}

fn copy_source_tree(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination).map_err(|error| LiveError::io(destination, error))?;
    for (path, relative) in source_files(source)? {
        let destination = destination.join(relative);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(|error| LiveError::io(parent, error))?;
        }
        fs::copy(&path, &destination).map_err(|error| LiveError::io(&path, error))?;
    }
    Ok(())
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
            COMPONENT_PYTHON_INSTALL_VERSION,
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
        let provenance = check_source(root.path(), &source()).expect("portable dependency");
        assert_eq!(provenance.profile, "wasi-component");
        assert_eq!(provenance.guest_contract, GUEST_CONTRACT);
        assert_eq!(provenance.target_abi, TARGET_ABI);
        assert_eq!(provenance.runtime.version, COMPONENT_PYTHON_VERSION);
        assert_eq!(provenance.componentizer.version, COMPONENTIZE_PY_VERSION);
        let distribution = provenance
            .componentizer
            .distribution
            .as_ref()
            .expect("host distribution");
        assert!(distribution
            .url
            .starts_with("https://github.com/Hologram-Technologies/hologram-live/releases/download/componentizer-v0.25.0-hologram.5/"));
        assert_eq!(distribution.sha256.len(), 64);
        assert_eq!(
            provenance.componentizer.patch_set,
            Some(componentizer_patch_set())
        );
        assert!(provenance.componentizer_runner.is_none());
        assert!(provenance.dependency_installer.is_none());
        assert!(provenance.output.is_none());
        assert!(!provenance.reproducibility.reproducible);
        assert_eq!(provenance.dependencies.len(), 1);
        assert_eq!(provenance.dependencies[0].name, "six");
        assert_eq!(provenance.inputs.len(), 3);
        assert_eq!(provenance.inputs[0].path, "project/pyproject.toml");
        assert_eq!(provenance.inputs[1].path, "project/uv.lock");
        assert_eq!(provenance.inputs[2].path, "project/src");
        assert!(provenance
            .inputs
            .iter()
            .all(|input| input.sha256.len() == 64));
    }

    #[test]
    fn every_release_host_has_an_exact_componentizer_wheel() {
        let targets = [
            (
                "linux",
                "x86_64",
                "manylinux_2_28_x86_64.whl",
                "1285eeb7cec8408153523016228f2afe577357419101373dc94b77fe54d7973f",
            ),
            (
                "linux",
                "aarch64",
                "manylinux_2_28_aarch64.whl",
                "72e6ae13ff1b597e2e7adfafb80b562463acfbedb69603e3f0b36f83c895c365",
            ),
            (
                "macos",
                "x86_64",
                "macosx_10_12_x86_64.whl",
                "4653f85787ce1fd8f21abeb3ed07f940367a6a8f16df7bc7279131a0252a4da1",
            ),
            (
                "macos",
                "aarch64",
                "macosx_11_0_arm64.whl",
                "eb9a6ed5c5d93ef949bcf2682b64b9097d1fa13b8f87fe3aabe54be7415559f8",
            ),
            (
                "windows",
                "x86_64",
                "win_amd64.whl",
                "fdf254a2d3ec235921a4a7a62d63b0c39e7b7da4444f08ba6cad730c35965a39",
            ),
        ];
        for (os, arch, wheel_suffix, sha256) in targets {
            let distribution =
                componentizer_distribution(os, arch).expect("release host distribution");
            assert!(distribution.url.starts_with(&format!(
                "https://github.com/Hologram-Technologies/hologram-live/releases/download/{COMPONENTIZER_RELEASE_TAG}/"
            )));
            assert!(distribution.url.ends_with(wheel_suffix));
            assert_eq!(distribution.sha256, sha256);
            assert!(distribution
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()));
        }
    }

    #[test]
    fn componentizer_patch_set_has_an_immutable_identity_and_contract() {
        let patch_set = componentizer_patch_set();
        assert_eq!(patch_set.release_tag, COMPONENTIZER_RELEASE_TAG);
        assert_eq!(patch_set.release_url, COMPONENTIZER_RELEASE_URL);
        assert_eq!(patch_set.manifest_url, COMPONENTIZER_PATCHSET_URL);
        assert_eq!(patch_set.manifest_sha256, COMPONENTIZER_PATCHSET_SHA256);
        assert_eq!(
            patch_set.determinism_contract,
            COMPONENTIZER_DETERMINISM_CONTRACT
        );
    }

    #[test]
    fn unsupported_componentizer_host_fails_closed() {
        let error = componentizer_distribution("freebsd", "x86_64")
            .expect_err("unsupported host must fail");
        assert_eq!(error.code(), "LIVE_CAPABILITY_MISSING");
        assert!(error.to_string().contains("freebsd/x86_64"), "{error}");
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

    #[test]
    fn source_tree_digest_is_ordered_and_content_addressed() {
        let first = tempfile::tempdir().expect("first tree");
        let second = tempfile::tempdir().expect("second tree");
        for root in [first.path(), second.path()] {
            fs::create_dir_all(root.join("package/nested")).expect("nested source");
        }
        fs::write(first.path().join("package/z.py"), b"z = 1\n").expect("z first");
        fs::write(first.path().join("package/nested/a.py"), b"a = 1\n").expect("a first");
        fs::write(second.path().join("package/nested/a.py"), b"a = 1\n").expect("a second");
        fs::write(second.path().join("package/z.py"), b"z = 1\n").expect("z second");

        let first_hash = sha256_source_tree(first.path()).expect("hash first");
        let second_hash = sha256_source_tree(second.path()).expect("hash second");
        assert_eq!(first_hash, second_hash);

        let staged = tempfile::tempdir().expect("staged tree");
        copy_source_tree(first.path(), staged.path()).expect("stage source tree");
        assert_eq!(
            first_hash,
            sha256_source_tree(staged.path()).expect("hash staged tree")
        );

        fs::write(second.path().join("package/z.py"), b"z = 2\n").expect("change source");
        assert_ne!(
            first_hash,
            sha256_source_tree(second.path()).expect("hash changed tree")
        );
    }

    #[cfg(unix)]
    #[test]
    fn nested_source_symlink_is_rejected() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().expect("source tree");
        fs::write(root.path().join("target.py"), b"value = 1\n").expect("target");
        symlink("target.py", root.path().join("alias.py")).expect("source symlink");
        let error = sha256_source_tree(root.path()).expect_err("symlink rejected");
        assert_eq!(error.code(), "LIVE_CONFIG_INVALID");
        assert!(error.to_string().contains("alias.py"), "{error}");
    }
}
