//! Python rootfs compilation and direct execution.
//!
//! The payload is a small Hologram envelope around a Docker image archive.
//! Python and its locked dependencies execute in an OCI container, never in
//! the Hologram host process. The `RootfsImage` layer keeps the archive contract
//! independent from the current container provider so a microVM can consume
//! the same logical layer in a later milestone.

use crate::application_plan::ProviderContext;
use crate::error::{LiveError, Result};
use crate::holo_provider::{
    LayerCompletion, LayerInvocation, LayerPrepareContext, LayerProvider, LayerRuntimeStatus,
    PreparedLayer, ProviderTarget,
};
use hologram::space::LayerKind;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

const MAGIC: &[u8; 8] = b"HOLOPYR1";
const BUNDLE_SCHEMA_VERSION: u16 = 2;
const LEGACY_BUNDLE_SCHEMA_VERSION: u16 = 1;
const UV_VERSION: &str = "0.11.8";
const OCI_COMPRESSION_LEVEL: i32 = 3;
const MAX_METADATA_BYTES: usize = 64 * 1024;
const MAX_INPUT_BYTES: usize = 16 * 1024 * 1024;
const MAX_OUTPUT_BYTES: usize = 16 * 1024 * 1024;
const MAX_DIAGNOSTIC_BYTES: usize = 64 * 1024;
const MAX_IMAGE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const RUN_TIMEOUT: Duration = Duration::from_secs(30);
static RUN_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PythonRootfsSource {
    pub project: PathBuf,
    pub entry: String,
    pub lock: PathBuf,
    pub profile: PythonProfile,
    #[serde(default = "default_base")]
    pub base: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PythonProfile {
    Rootfs,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct BundleMetadata {
    schema_version: u16,
    provider: String,
    image_tag: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    image_id: Option<String>,
    entry: String,
    arch: String,
    image_uncompressed_bytes: u64,
}

pub struct PythonRunOutcome {
    pub outputs: Vec<Vec<u8>>,
    pub elapsed_micros: u64,
    pub resident_bytes: usize,
}

#[derive(Debug)]
struct SourceInputs {
    pyproject: PathBuf,
    lock: PathBuf,
    source_dir: PathBuf,
}

pub fn validate_source(source: &PythonRootfsSource, arch: Option<&str>) -> Result<()> {
    if source.profile != PythonProfile::Rootfs {
        return Err(LiveError::Config(
            "Python source currently supports only the rootfs profile".to_owned(),
        ));
    }
    validate_entry(&source.entry)?;
    validate_base(&source.base)?;
    normalize_arch(
        arch.ok_or_else(|| LiveError::Config("Python rootfs layers require arch".to_owned()))?,
    )?;
    if source.project.is_absolute() || source.lock.is_absolute() {
        return Err(LiveError::Config(
            "Python project and lock paths must be relative to hologram.json".to_owned(),
        ));
    }
    Ok(())
}

pub fn compile(root: &Path, source: &PythonRootfsSource, arch: &str) -> Result<Vec<u8>> {
    validate_source(source, Some(arch))?;
    ensure_docker("compile a Python rootfs")?;
    let inputs = resolve_inputs(root, source)?;

    let staging = tempfile::tempdir().map_err(LiveError::from)?;
    fs::copy(&inputs.pyproject, staging.path().join("pyproject.toml"))
        .map_err(|error| LiveError::io(&inputs.pyproject, error))?;
    fs::copy(&inputs.lock, staging.path().join("uv.lock"))
        .map_err(|error| LiveError::io(&inputs.lock, error))?;
    copy_tree(&inputs.source_dir, &staging.path().join("src"))?;
    let runner_dir = staging.path().join(".hologram");
    fs::create_dir(&runner_dir).map_err(|error| LiveError::io(&runner_dir, error))?;
    fs::write(runner_dir.join("runner.py"), runner_source())
        .map_err(|error| LiveError::io(&runner_dir, error))?;

    let image_tag = image_tag(
        &inputs.pyproject,
        &inputs.lock,
        &inputs.source_dir,
        source,
        arch,
    )?;
    let dockerfile = dockerfile(&source.base, &source.entry);
    fs::write(staging.path().join("Dockerfile"), dockerfile)
        .map_err(|error| LiveError::io(staging.path(), error))?;

    let platform = docker_platform(arch)?;
    let build = Command::new("docker")
        .args([
            "build",
            "--platform",
            platform,
            "--tag",
            &image_tag,
            "--file",
            "Dockerfile",
            ".",
        ])
        .current_dir(staging.path())
        .env("SOURCE_DATE_EPOCH", "0")
        .output()
        .map_err(|error| LiveError::Io(format!("start Docker build: {error}")))?;
    command_succeeded("Docker build", &build)?;

    let image_path = staging.path().join("image.tar");
    let save = Command::new("docker")
        .args(["image", "save", "--output"])
        .arg(&image_path)
        .arg(&image_tag)
        .output()
        .map_err(|error| LiveError::Io(format!("start Docker image save: {error}")))?;
    command_succeeded("Docker image save", &save)?;
    let image_bytes = fs::metadata(&image_path)
        .map_err(|error| LiveError::io(&image_path, error))?
        .len();
    let image = fs::File::open(&image_path).map_err(|error| LiveError::io(&image_path, error))?;
    let compressed = zstd::stream::encode_all(image, OCI_COMPRESSION_LEVEL)
        .map_err(|error| LiveError::Io(format!("compress Python OCI image: {error}")))?;
    let image_id = inspect_image_id(&image_tag)?.ok_or_else(|| {
        LiveError::Conflict(format!(
            "Docker build completed but image {image_tag} is unavailable"
        ))
    })?;
    encode_bundle(
        &BundleMetadata {
            schema_version: BUNDLE_SCHEMA_VERSION,
            provider: "oci-docker-zstd-v1".to_owned(),
            image_tag,
            image_id: Some(image_id),
            entry: source.entry.clone(),
            arch: normalize_arch(arch)?.to_owned(),
            image_uncompressed_bytes: image_bytes,
        },
        &compressed,
    )
}

pub fn check_source(root: &Path, source: &PythonRootfsSource, arch: &str) -> Result<()> {
    validate_source(source, Some(arch))?;
    resolve_inputs(root, source).map(|_| ())
}

pub fn execute(
    bundle: &[u8],
    layer_entry: &str,
    layer_arch: &str,
    inputs: &[Vec<u8>],
) -> Result<PythonRunOutcome> {
    let started = Instant::now();
    let (metadata, compressed_image) = decode_bundle(bundle)?;
    let arch = validate_bundle_metadata(&metadata, layer_entry, layer_arch)?;
    ensure_docker("execute a Python rootfs")?;
    if !cached_image_matches(&metadata)? {
        load_embedded_image(&metadata, compressed_image)?;
    }

    let mut outputs = Vec::with_capacity(inputs.len());
    for input in inputs {
        if input.len() > MAX_INPUT_BYTES {
            return Err(LiveError::Protocol(format!(
                "Python input is {} bytes; limit is {MAX_INPUT_BYTES}",
                input.len()
            )));
        }
        outputs.push(run_container(&metadata, arch, input)?);
    }
    Ok(PythonRunOutcome {
        outputs,
        elapsed_micros: u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX),
        resident_bytes: bundle.len(),
    })
}

pub struct PythonRootfsProvider;

#[tonic::async_trait]
impl LayerProvider for PythonRootfsProvider {
    fn kind(&self) -> LayerKind {
        LayerKind::RootfsImage
    }

    fn contract(&self) -> Option<&'static str> {
        None
    }

    fn name(&self) -> &'static str {
        "python-oci-direct"
    }

    fn availability(
        &self,
        context: &ProviderContext<'_>,
        target: ProviderTarget,
    ) -> Result<(), String> {
        if target != ProviderTarget::Direct {
            return Err("Python OCI rootfs execution is direct-only".to_owned());
        }
        if !is_python_rootfs(context.content) {
            return Err(format!(
                "direct execution has no compatible rootfs provider for entry {}",
                context.entry
            ));
        }
        Ok(())
    }

    async fn prepare(&self, context: LayerPrepareContext) -> Result<Arc<dyn PreparedLayer>> {
        if context.target != ProviderTarget::Direct {
            return Err(LiveError::Capability(
                "Python OCI rootfs execution is direct-only".to_owned(),
            ));
        }
        validate_bundle_for_layer(
            &context.layer.content,
            &context.layer.entry,
            &context.layer.aux,
        )?;
        Ok(Arc::new(PreparedPythonRootfs {
            position: context.layer.position,
            content: context.layer.content,
            entry: context.layer.entry,
            arch: context.layer.aux,
            running: AtomicBool::new(false),
            processed: AtomicUsize::new(0),
        }))
    }
}

struct PreparedPythonRootfs {
    position: u32,
    content: Arc<[u8]>,
    entry: String,
    arch: String,
    running: AtomicBool,
    processed: AtomicUsize,
}

#[tonic::async_trait]
impl PreparedLayer for PreparedPythonRootfs {
    fn position(&self) -> u32 {
        self.position
    }

    async fn start(&self) -> Result<()> {
        tokio::task::spawn_blocking(|| ensure_docker("execute a Python rootfs"))
            .await
            .map_err(|error| {
                LiveError::Conflict(format!("join Python provider start: {error}"))
            })??;
        self.running.store(true, Ordering::Release);
        Ok(())
    }

    async fn invoke(&self, inputs: Vec<Vec<u8>>) -> Result<LayerInvocation> {
        if !self.running.load(Ordering::Acquire) {
            return Err(LiveError::Conflict(format!(
                "Python rootfs layer {} is not running",
                self.position
            )));
        }
        let content = self.content.clone();
        let entry = self.entry.clone();
        let arch = self.arch.clone();
        let outcome =
            tokio::task::spawn_blocking(move || execute(&content, &entry, &arch, &inputs))
                .await
                .map_err(|error| {
                    LiveError::Conflict(format!("join Python provider invocation: {error}"))
                })??;
        self.processed.fetch_add(1, Ordering::Relaxed);
        Ok(LayerInvocation {
            outputs: outcome.outputs,
            completion: LayerCompletion::Exited { code: 0 },
            elapsed_micros: outcome.elapsed_micros,
        })
    }

    async fn stop(&self) -> Result<()> {
        self.running.store(false, Ordering::Release);
        Ok(())
    }

    fn status(&self) -> LayerRuntimeStatus {
        LayerRuntimeStatus {
            resident_bytes: self.content.len(),
            queued: 0,
            processed: self.processed.load(Ordering::Relaxed),
        }
    }
}

fn validate_bundle_for_layer(bundle: &[u8], layer_entry: &str, layer_arch: &str) -> Result<()> {
    let (metadata, _) = decode_bundle(bundle)?;
    validate_bundle_metadata(&metadata, layer_entry, layer_arch).map(|_| ())
}

fn validate_bundle_metadata(
    metadata: &BundleMetadata,
    layer_entry: &str,
    layer_arch: &str,
) -> Result<&'static str> {
    if metadata.entry != layer_entry {
        return Err(LiveError::InvalidHolo(format!(
            "Python rootfs entry {} does not match layer entry {layer_entry}",
            metadata.entry
        )));
    }
    let arch = normalize_arch(layer_arch)?;
    if metadata.arch != arch {
        return Err(LiveError::InvalidHolo(format!(
            "Python rootfs architecture {} does not match layer architecture {arch}",
            metadata.arch
        )));
    }
    let host = host_arch()?;
    if arch != host {
        return Err(LiveError::Capability(format!(
            "Python rootfs requires {arch}, but this host is {host}; compile for the host architecture"
        )));
    }
    Ok(arch)
}

pub fn is_python_rootfs(bytes: &[u8]) -> bool {
    bytes.starts_with(MAGIC)
}

pub fn canonical_arch(arch: &str) -> Result<&'static str> {
    normalize_arch(arch)
}

fn run_container(metadata: &BundleMetadata, arch: &str, input: &[u8]) -> Result<Vec<u8>> {
    let container_name = format!(
        "hologram-python-{}-{}",
        std::process::id(),
        RUN_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    let mut child = Command::new("docker")
        .args([
            "run",
            "--rm",
            "--name",
            &container_name,
            "--interactive",
            "--platform",
            docker_platform(arch)?,
            "--network",
            "none",
            "--read-only",
            "--cap-drop",
            "ALL",
            "--security-opt",
            "no-new-privileges",
            "--pids-limit",
            "64",
            "--memory",
            "1g",
            "--cpus",
            "1",
            "--tmpfs",
            "/tmp:rw,noexec,nosuid,size=64m",
            &metadata.image_tag,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| LiveError::Io(format!("start Python rootfs container: {error}")))?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| LiveError::Io("open Python rootfs stdin".to_owned()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| LiveError::Io("open Python rootfs stdout".to_owned()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| LiveError::Io("open Python rootfs stderr".to_owned()))?;
    let input = input.to_vec();
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let stdout_reader = thread::spawn(move || read_bounded(stdout, MAX_OUTPUT_BYTES));
        let stderr_reader = thread::spawn(move || read_bounded(stderr, MAX_DIAGNOSTIC_BYTES));
        let result: Result<(std::process::ExitStatus, Vec<u8>, Vec<u8>)> = (|| {
            let mut stdin = stdin;
            stdin
                .write_all(&input)
                .map_err(|error| LiveError::Io(format!("write Python rootfs input: {error}")))?;
            drop(stdin);
            let status = child
                .wait()
                .map_err(|error| LiveError::Io(format!("wait for Python rootfs: {error}")))?;
            let stdout = stdout_reader
                .join()
                .map_err(|_| LiveError::Io("read Python rootfs stdout".to_owned()))??;
            let stderr = stderr_reader
                .join()
                .map_err(|_| LiveError::Io("read Python rootfs stderr".to_owned()))??;
            Ok((status, stdout, stderr))
        })();
        let _ = sender.send(result);
    });
    let (status, stdout, stderr) = match receiver.recv_timeout(RUN_TIMEOUT) {
        Ok(result) => result?,
        Err(mpsc::RecvTimeoutError::Timeout) => {
            let _ = Command::new("docker")
                .args(["container", "rm", "--force", &container_name])
                .output();
            return Err(LiveError::Conflict(format!(
                "Python entrypoint {} exceeded the {} second execution limit",
                metadata.entry,
                RUN_TIMEOUT.as_secs()
            )));
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            return Err(LiveError::Io(
                "Python rootfs execution worker stopped unexpectedly".to_owned(),
            ));
        }
    };
    if stdout.len() > MAX_OUTPUT_BYTES {
        return Err(LiveError::Protocol(format!(
            "Python output is {} bytes; limit is {MAX_OUTPUT_BYTES}",
            stdout.len()
        )));
    }
    if !status.success() {
        return Err(LiveError::Protocol(format!(
            "Python entrypoint {} failed: {}",
            metadata.entry,
            diagnostic(&stderr)
        )));
    }
    Ok(stdout)
}

fn read_bounded(reader: impl Read, limit: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader
        .take(u64::try_from(limit).unwrap_or(u64::MAX) + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| LiveError::Io(format!("read Python rootfs output: {error}")))?;
    Ok(bytes)
}

fn encode_bundle(metadata: &BundleMetadata, image: &[u8]) -> Result<Vec<u8>> {
    let encoded = serde_json::to_vec(metadata)?;
    if encoded.len() > MAX_METADATA_BYTES {
        return Err(LiveError::Config(
            "Python rootfs metadata exceeds its size limit".to_owned(),
        ));
    }
    let length = u32::try_from(encoded.len())
        .map_err(|_| LiveError::Config("Python rootfs metadata is too large".to_owned()))?;
    let mut bundle = Vec::with_capacity(MAGIC.len() + 4 + encoded.len() + image.len());
    bundle.extend_from_slice(MAGIC);
    bundle.extend_from_slice(&length.to_le_bytes());
    bundle.extend_from_slice(&encoded);
    bundle.extend_from_slice(image);
    Ok(bundle)
}

fn decode_bundle(bundle: &[u8]) -> Result<(BundleMetadata, &[u8])> {
    if bundle.len() < MAGIC.len() + 4 || !bundle.starts_with(MAGIC) {
        return Err(LiveError::InvalidHolo(
            "rootfs payload is not a Hologram Python OCI bundle".to_owned(),
        ));
    }
    let length = usize::try_from(u32::from_le_bytes(
        bundle[MAGIC.len()..MAGIC.len() + 4]
            .try_into()
            .map_err(|_| LiveError::InvalidHolo("invalid Python metadata length".to_owned()))?,
    ))
    .map_err(|_| LiveError::InvalidHolo("invalid Python metadata length".to_owned()))?;
    if length > MAX_METADATA_BYTES || MAGIC.len() + 4 + length >= bundle.len() {
        return Err(LiveError::InvalidHolo(
            "invalid Python rootfs metadata boundary".to_owned(),
        ));
    }
    let start = MAGIC.len() + 4;
    let metadata: BundleMetadata = serde_json::from_slice(&bundle[start..start + length])
        .map_err(|error| LiveError::InvalidHolo(format!("decode Python metadata: {error}")))?;
    if !matches!(
        metadata.schema_version,
        LEGACY_BUNDLE_SCHEMA_VERSION | BUNDLE_SCHEMA_VERSION
    ) || metadata.provider != "oci-docker-zstd-v1"
    {
        return Err(LiveError::Capability(format!(
            "unsupported Python rootfs provider {} schema {}",
            metadata.provider, metadata.schema_version
        )));
    }
    validate_entry(&metadata.entry).map_err(|error| LiveError::InvalidHolo(error.to_string()))?;
    normalize_arch(&metadata.arch).map_err(|error| LiveError::InvalidHolo(error.to_string()))?;
    if metadata.schema_version == BUNDLE_SCHEMA_VERSION
        && !metadata.image_id.as_deref().is_some_and(valid_image_id)
    {
        return Err(LiveError::InvalidHolo(
            "Python rootfs bundle is missing a valid Docker image ID".to_owned(),
        ));
    }
    if metadata.image_uncompressed_bytes == 0 || metadata.image_uncompressed_bytes > MAX_IMAGE_BYTES
    {
        return Err(LiveError::InvalidHolo(format!(
            "invalid Python OCI image size {}",
            metadata.image_uncompressed_bytes
        )));
    }
    Ok((metadata, &bundle[start + length..]))
}

fn decompress_image(compressed: &[u8], expected: u64) -> Result<Vec<u8>> {
    let decoder = zstd::stream::Decoder::new(compressed)
        .map_err(|error| LiveError::InvalidHolo(format!("open Python OCI image: {error}")))?;
    let mut image = Vec::new();
    decoder
        .take(MAX_IMAGE_BYTES + 1)
        .read_to_end(&mut image)
        .map_err(|error| LiveError::InvalidHolo(format!("decompress Python OCI image: {error}")))?;
    let actual = u64::try_from(image.len()).unwrap_or(u64::MAX);
    if actual != expected {
        return Err(LiveError::InvalidHolo(format!(
            "Python OCI image expanded to {actual} bytes; expected {expected}"
        )));
    }
    Ok(image)
}

fn ensure_docker(action: &str) -> Result<()> {
    let output = Command::new("docker")
        .args(["version", "--format", "{{.Server.Version}}"])
        .output()
        .map_err(|error| {
            LiveError::Capability(format!(
                "{action} requires the Docker CLI and a running Docker-compatible engine: {error}"
            ))
        })?;
    if !output.status.success() || output.stdout.is_empty() {
        return Err(LiveError::Capability(format!(
            "{action} requires a running Docker-compatible engine: {}",
            diagnostic(&output.stderr)
        )));
    }
    Ok(())
}

fn cached_image_matches(metadata: &BundleMetadata) -> Result<bool> {
    let Some(expected) = metadata.image_id.as_deref() else {
        return Ok(false);
    };
    Ok(inspect_image_id(&metadata.image_tag)?.as_deref() == Some(expected))
}

fn inspect_image_id(image_tag: &str) -> Result<Option<String>> {
    let output = Command::new("docker")
        .args(["image", "inspect", "--format", "{{.Id}}", image_tag])
        .output()
        .map_err(|error| LiveError::Io(format!("inspect Docker image {image_tag}: {error}")))?;
    if !output.status.success() {
        return Ok(None);
    }
    let image_id = String::from_utf8(output.stdout)
        .map_err(|error| LiveError::Protocol(format!("decode Docker image ID: {error}")))?;
    let image_id = image_id.trim();
    if valid_image_id(image_id) {
        Ok(Some(image_id.to_owned()))
    } else {
        Err(LiveError::Protocol(format!(
            "Docker returned invalid image ID {image_id} for {image_tag}"
        )))
    }
}

fn load_embedded_image(metadata: &BundleMetadata, compressed_image: &[u8]) -> Result<()> {
    let image = decompress_image(compressed_image, metadata.image_uncompressed_bytes)?;
    let image_file = tempfile::NamedTempFile::new().map_err(LiveError::from)?;
    fs::write(image_file.path(), image).map_err(|error| LiveError::io(image_file.path(), error))?;
    let load = Command::new("docker")
        .args(["image", "load", "--input"])
        .arg(image_file.path())
        .output()
        .map_err(|error| LiveError::Io(format!("start Docker image load: {error}")))?;
    command_succeeded("Docker image load", &load)?;
    if let Some(expected) = metadata.image_id.as_deref() {
        let actual = inspect_image_id(&metadata.image_tag)?.ok_or_else(|| {
            LiveError::InvalidHolo(format!(
                "embedded Python image did not load tag {}",
                metadata.image_tag
            ))
        })?;
        if actual != expected {
            return Err(LiveError::InvalidHolo(format!(
                "embedded Python image loaded as {actual}; expected {expected}"
            )));
        }
    }
    Ok(())
}

fn command_succeeded(name: &str, output: &std::process::Output) -> Result<()> {
    if output.status.success() {
        Ok(())
    } else {
        Err(LiveError::Conflict(format!(
            "{name} failed: {}",
            diagnostic(&output.stderr)
        )))
    }
}

fn dockerfile(base: &str, entry: &str) -> String {
    format!(
        "FROM {base}\n\
         ENV PYTHONDONTWRITEBYTECODE=1 PYTHONUNBUFFERED=1 UV_COMPILE_BYTECODE=0\n\
         WORKDIR /app\n\
         RUN python -m pip install --no-cache-dir uv=={UV_VERSION}\n\
         COPY pyproject.toml uv.lock ./\n\
         COPY src ./src\n\
         RUN uv sync --locked --no-dev --no-editable\n\
         COPY .hologram/runner.py /hologram/runner.py\n\
         ENV PATH=\"/app/.venv/bin:$PATH\" HOLOGRAM_PYTHON_ENTRY=\"{entry}\"\n\
         ENTRYPOINT [\"python\",\"/hologram/runner.py\"]\n"
    )
}

fn runner_source() -> &'static str {
    r#"import importlib
import os
import sys

module_name, function_name = os.environ["HOLOGRAM_PYTHON_ENTRY"].split(":", 1)
function = getattr(importlib.import_module(module_name), function_name)
result = function(sys.stdin.buffer.read())
if not isinstance(result, bytes):
    raise TypeError("Hologram Python entrypoint must return bytes")
sys.stdout.buffer.write(result)
"#
}

fn image_tag(
    pyproject: &Path,
    lock: &Path,
    source_dir: &Path,
    source: &PythonRootfsSource,
    arch: &str,
) -> Result<String> {
    let mut hasher = blake3::Hasher::new();
    hasher.update(&fs::read(pyproject).map_err(|error| LiveError::io(pyproject, error))?);
    hasher.update(&fs::read(lock).map_err(|error| LiveError::io(lock, error))?);
    hasher.update(&serde_json::to_vec(source)?);
    hasher.update(normalize_arch(arch)?.as_bytes());
    hash_tree(source_dir, source_dir, &mut hasher)?;
    let digest = hasher.finalize().to_hex();
    Ok(format!("hologram-python-{}:local", &digest[..24]))
}

fn hash_tree(root: &Path, path: &Path, hasher: &mut blake3::Hasher) -> Result<()> {
    let mut entries = fs::read_dir(path)
        .map_err(|error| LiveError::io(path, error))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|error| LiveError::io(path, error))?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let entry_path = entry.path();
        let metadata =
            fs::symlink_metadata(&entry_path).map_err(|error| LiveError::io(&entry_path, error))?;
        if metadata.file_type().is_symlink() {
            return Err(LiveError::Config(format!(
                "Python source may not contain symlink {}",
                entry_path.display()
            )));
        }
        let relative = entry_path
            .strip_prefix(root)
            .map_err(|error| LiveError::Config(format!("resolve Python source path: {error}")))?;
        hasher.update(relative.to_string_lossy().as_bytes());
        if metadata.is_dir() {
            hash_tree(root, &entry_path, hasher)?;
        } else if metadata.is_file() {
            hasher
                .update(&fs::read(&entry_path).map_err(|error| LiveError::io(&entry_path, error))?);
        } else {
            return Err(LiveError::Config(format!(
                "unsupported Python source file {}",
                entry_path.display()
            )));
        }
    }
    Ok(())
}

fn copy_tree(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination).map_err(|error| LiveError::io(destination, error))?;
    let mut entries = fs::read_dir(source)
        .map_err(|error| LiveError::io(source, error))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|error| LiveError::io(source, error))?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let from = entry.path();
        let to = destination.join(entry.file_name());
        let metadata = fs::symlink_metadata(&from).map_err(|error| LiveError::io(&from, error))?;
        if metadata.file_type().is_symlink() {
            return Err(LiveError::Config(format!(
                "Python source may not contain symlink {}",
                from.display()
            )));
        }
        if metadata.is_dir() {
            copy_tree(&from, &to)?;
        } else if metadata.is_file() {
            fs::copy(&from, &to).map_err(|error| LiveError::io(&from, error))?;
        } else {
            return Err(LiveError::Config(format!(
                "unsupported Python source file {}",
                from.display()
            )));
        }
    }
    Ok(())
}

fn validate_entry(entry: &str) -> Result<()> {
    let Some((module, function)) = entry.split_once(':') else {
        return Err(LiveError::Config(
            "Python entry must use module:function syntax".to_owned(),
        ));
    };
    if !valid_module(module) || !valid_identifier(function) {
        return Err(LiveError::Config(format!(
            "invalid Python entry {entry}; expected module.path:function"
        )));
    }
    Ok(())
}

fn valid_module(value: &str) -> bool {
    !value.is_empty() && value.split('.').all(valid_identifier)
}

fn valid_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    chars
        .next()
        .is_some_and(|character| character == '_' || character.is_ascii_alphabetic())
        && chars.all(|character| character == '_' || character.is_ascii_alphanumeric())
}

fn validate_base(base: &str) -> Result<()> {
    if base.is_empty()
        || !base
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "-._/:@".contains(character))
    {
        return Err(LiveError::Config(format!(
            "invalid Python OCI base image {base}"
        )));
    }
    Ok(())
}

fn valid_image_id(image_id: &str) -> bool {
    image_id.strip_prefix("sha256:").is_some_and(|digest| {
        digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
    })
}

fn normalize_arch(arch: &str) -> Result<&'static str> {
    match arch {
        "host" => host_arch(),
        "arm64" | "aarch64" => Ok("arm64"),
        "x86_64" | "amd64" => Ok("x86_64"),
        _ => Err(LiveError::Config(format!(
            "unsupported Python rootfs architecture {arch}; expected host, arm64, or x86_64"
        ))),
    }
}

fn host_arch() -> Result<&'static str> {
    match std::env::consts::ARCH {
        "arm64" | "aarch64" => Ok("arm64"),
        "x86_64" | "amd64" => Ok("x86_64"),
        arch => Err(LiveError::Capability(format!(
            "Python rootfs execution is unsupported on host architecture {arch}"
        ))),
    }
}

fn docker_platform(arch: &str) -> Result<&'static str> {
    match normalize_arch(arch)? {
        "arm64" => Ok("linux/arm64"),
        "x86_64" => Ok("linux/amd64"),
        _ => unreachable!(),
    }
}

fn require_file(path: &Path, label: &str) -> Result<()> {
    if path.is_file() {
        Ok(())
    } else {
        Err(LiveError::Config(format!(
            "{label} {} is missing or is not a file",
            path.display()
        )))
    }
}

fn require_directory(path: &Path, label: &str) -> Result<()> {
    if path.is_dir() {
        Ok(())
    } else {
        Err(LiveError::Config(format!(
            "{label} {} is missing or is not a directory",
            path.display()
        )))
    }
}

fn resolve_inputs(root: &Path, source: &PythonRootfsSource) -> Result<SourceInputs> {
    let root = root
        .canonicalize()
        .map_err(|error| LiveError::io(root, error))?;
    let project = root.join(&source.project);
    reject_symlink(&project, "Python project")?;
    let project = project
        .canonicalize()
        .map_err(|error| LiveError::io(&project, error))?;
    if !project.starts_with(&root) {
        return Err(LiveError::Config(format!(
            "Python project {} escapes the manifest directory {}",
            project.display(),
            root.display()
        )));
    }
    let pyproject = project.join("pyproject.toml");
    let lock = project.join(&source.lock);
    let source_dir = project.join("src");
    require_file(&pyproject, "Python project metadata")?;
    require_file(&lock, "Python lock file")?;
    require_directory(&source_dir, "Python src directory")?;
    reject_symlink(&pyproject, "Python project metadata")?;
    reject_symlink(&lock, "Python lock file")?;
    reject_symlink(&source_dir, "Python src directory")?;
    let inputs = SourceInputs {
        pyproject: pyproject
            .canonicalize()
            .map_err(|error| LiveError::io(&pyproject, error))?,
        lock: lock
            .canonicalize()
            .map_err(|error| LiveError::io(&lock, error))?,
        source_dir: source_dir
            .canonicalize()
            .map_err(|error| LiveError::io(&source_dir, error))?,
    };
    for input in [&inputs.pyproject, &inputs.lock, &inputs.source_dir] {
        if !input.starts_with(&project) {
            return Err(LiveError::Config(format!(
                "Python input {} escapes project directory {}",
                input.display(),
                project.display()
            )));
        }
    }
    Ok(inputs)
}

fn reject_symlink(path: &Path, label: &str) -> Result<()> {
    let metadata = fs::symlink_metadata(path).map_err(|error| LiveError::io(path, error))?;
    if metadata.file_type().is_symlink() {
        Err(LiveError::Config(format!(
            "{label} {} may not be a symlink",
            path.display()
        )))
    } else {
        Ok(())
    }
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

fn default_base() -> String {
    "python:3.12-slim".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metadata() -> BundleMetadata {
        BundleMetadata {
            schema_version: BUNDLE_SCHEMA_VERSION,
            provider: "oci-docker-zstd-v1".to_owned(),
            image_tag: "hologram-python-test:local".to_owned(),
            image_id: Some(format!("sha256:{}", "a".repeat(64))),
            entry: "example:main".to_owned(),
            arch: "arm64".to_owned(),
            image_uncompressed_bytes: 9,
        }
    }

    #[test]
    fn bundle_round_trips() {
        let compressed = zstd::stream::encode_all(b"image tar".as_slice(), 1).expect("compress");
        let bundle = encode_bundle(&metadata(), &compressed).expect("encode");
        let (decoded, image) = decode_bundle(&bundle).expect("decode");
        assert_eq!(decoded, metadata());
        assert_eq!(
            decompress_image(image, 9).expect("decompress"),
            b"image tar"
        );
    }

    #[test]
    fn legacy_bundle_without_an_image_id_still_decodes() {
        let mut legacy = metadata();
        legacy.schema_version = LEGACY_BUNDLE_SCHEMA_VERSION;
        legacy.image_id = None;
        let compressed = zstd::stream::encode_all(b"image tar".as_slice(), 1).expect("compress");
        let bundle = encode_bundle(&legacy, &compressed).expect("encode");
        let (decoded, _) = decode_bundle(&bundle).expect("decode");
        assert_eq!(decoded, legacy);
    }

    #[test]
    fn entrypoint_requires_module_and_function() {
        assert!(validate_entry("analytics.app:main").is_ok());
        assert!(validate_entry("analytics-app:main").is_err());
        assert!(validate_entry("analytics.app").is_err());
    }

    #[test]
    fn docker_image_ids_require_a_sha256_digest() {
        assert!(valid_image_id(&format!("sha256:{}", "a".repeat(64))));
        assert!(!valid_image_id("126a833f0669"));
        assert!(!valid_image_id(&format!("sha256:{}", "z".repeat(64))));
    }

    #[test]
    fn rejects_truncated_bundle() {
        let error = decode_bundle(b"HOLOPYR1\xff\xff\xff\xff").expect_err("invalid");
        assert_eq!(error.code(), "LIVE_HOLO_INVALID");
    }

    #[test]
    fn lock_file_may_not_escape_the_project() {
        let root = tempfile::tempdir().expect("root");
        let project = root.path().join("project");
        fs::create_dir_all(project.join("src")).expect("source directory");
        fs::write(project.join("pyproject.toml"), "[project]\nname='demo'\n")
            .expect("project metadata");
        fs::write(root.path().join("outside.lock"), "version = 1\n").expect("outside lock");
        let source = PythonRootfsSource {
            project: PathBuf::from("project"),
            entry: "demo:main".to_owned(),
            lock: PathBuf::from("../outside.lock"),
            profile: PythonProfile::Rootfs,
            base: default_base(),
        };
        let error = resolve_inputs(root.path(), &source).expect_err("escaping lock");
        assert!(error.to_string().contains("escapes project directory"));
    }
}
