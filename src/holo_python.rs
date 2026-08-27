//! Python rootfs compilation and direct execution.
//!
//! The payload is a small Hologram envelope around a normalized Docker image
//! archive.
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
use hologram::space::{address_bytes, LayerKind};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

const MAGIC: &[u8; 8] = b"HOLOPYR2";
const BUNDLE_SCHEMA_VERSION: u16 = 3;
const BUNDLE_PROVIDER: &str = "normalized-docker-archive-zstd-v1";
const ROOTFS_REPRESENTATION: &str = "normalized-docker-archive-v1";
const SOURCE_DATE_EPOCH: u64 = 0;
const UV_VERSION: &str = "0.11.8";
const OCI_COMPRESSION_LEVEL: i32 = 3;
const MAX_METADATA_BYTES: usize = 64 * 1024;
const MAX_INPUT_BYTES: usize = 16 * 1024 * 1024;
const MAX_OUTPUT_BYTES: usize = 16 * 1024 * 1024;
const MAX_DIAGNOSTIC_BYTES: usize = 64 * 1024;
const MAX_IMAGE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const RUN_TIMEOUT: Duration = Duration::from_secs(30);
const ROOTFS_MUTABLE_BASE_BLOCKER: &str = "compile --check does not contact registries, so this mutable Python rootfs base is not resolved until compilation; reproducibility is established only after the build binds it to an immutable digest";
static RUN_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PythonRootfsSource {
    pub project: PathBuf,
    pub entry: String,
    pub lock: PathBuf,
    pub profile: PythonProfile,
    #[serde(default = "default_base", skip_serializing_if = "is_default_base")]
    pub base: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PythonProfile {
    Rootfs,
    WasiComponent,
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BuildTool {
    pub name: &'static str,
    pub version: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DockerBuilder {
    pub name: &'static str,
    pub archive_format: &'static str,
    pub source_date_epoch: u64,
    pub cache_disabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_version: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BaseImageProvenance {
    /// The source recipe's user-supplied reference.
    pub reference: String,
    pub digest_pinned: bool,
    /// The immutable reference passed to Docker's `FROM` instruction.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_reference: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed_image_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BuildOutput {
    pub bundle_schema_version: u16,
    pub provider: &'static str,
    pub layer_kappa: String,
    pub byte_length: u64,
    pub image_id: String,
    pub image_uncompressed_bytes: u64,
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
    pub target_platform: String,
    pub build_host: crate::holo_python_component::BuildHost,
    pub compiler: BuildTool,
    pub base_image: BaseImageProvenance,
    pub dependency_installer: BuildTool,
    pub builder: DockerBuilder,
    pub inputs: Vec<crate::holo_python_component::BuildInput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<BuildOutput>,
    pub reproducibility: Reproducibility,
}

pub struct CompiledRootfs {
    pub bytes: Vec<u8>,
    pub provenance: BuildProvenance,
}

#[derive(Debug)]
pub(crate) struct SourceInputs {
    pub(crate) pyproject: PathBuf,
    pub(crate) lock: PathBuf,
    pub(crate) source_dir: PathBuf,
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

pub fn compile(
    root: &Path,
    source: &PythonRootfsSource,
    arch: &str,
    no_build_cache: bool,
) -> Result<CompiledRootfs> {
    validate_source(source, Some(arch))?;
    ensure_docker("compile a Python rootfs")?;
    let inputs = resolve_inputs(root, source)?;
    let mut provenance = build_provenance(source, &inputs, arch)?;
    let resolved_base = resolve_base_image(&source.base)?;
    provenance.base_image.resolved_reference = Some(resolved_base.clone());
    provenance.reproducibility = Reproducibility {
        reproducible: true,
        blocker: None,
    };

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
    let builder_dockerfile = builder_dockerfile(&resolved_base);
    fs::write(
        staging.path().join("Builder.Dockerfile"),
        builder_dockerfile,
    )
    .map_err(|error| LiveError::io(staging.path(), error))?;
    let runtime_dockerfile = runtime_dockerfile(&resolved_base, &source.entry);
    fs::write(staging.path().join("Dockerfile"), runtime_dockerfile)
        .map_err(|error| LiveError::io(staging.path(), error))?;

    let platform = docker_platform(arch)?;
    let source_date_epoch = format!("SOURCE_DATE_EPOCH={SOURCE_DATE_EPOCH}");
    let builder_tag = format!("{image_tag}-builder");
    let builder_build = Command::new("docker")
        .args(docker_build_args(
            platform,
            &builder_tag,
            "Builder.Dockerfile",
            &source_date_epoch,
            no_build_cache,
        ))
        .current_dir(staging.path())
        .env("SOURCE_DATE_EPOCH", "0")
        .output()
        .map_err(|error| LiveError::Io(format!("start Docker builder image: {error}")))?;
    command_succeeded("Docker builder image", &builder_build)?;
    let runtime_copy = copy_image_file(
        &builder_tag,
        "/runtime.tar",
        &staging.path().join("runtime.tar"),
    );
    remove_image_tag(&builder_tag);
    runtime_copy?;

    let build = Command::new("docker")
        .args(docker_build_args(
            platform,
            &image_tag,
            "Dockerfile",
            &source_date_epoch,
            no_build_cache,
        ))
        .current_dir(staging.path())
        .env("SOURCE_DATE_EPOCH", "0")
        .output()
        .map_err(|error| LiveError::Io(format!("start Docker build: {error}")))?;
    command_succeeded("Docker build", &build)?;
    provenance.builder = observed_docker_builder(no_build_cache)?;
    provenance.base_image.observed_image_id = inspect_image_id(&resolved_base)?;

    let raw_image_path = staging.path().join("image.raw.tar");
    let save = Command::new("docker")
        .args(["image", "save", "--output"])
        .arg(&raw_image_path)
        .arg(&image_tag)
        .output()
        .map_err(|error| LiveError::Io(format!("start Docker image save: {error}")))?;
    command_succeeded("Docker image save", &save)?;
    let observed_image_id = inspect_image_id(&image_tag)?.ok_or_else(|| {
        LiveError::Conflict(format!(
            "Docker build completed but image {image_tag} is unavailable"
        ))
    })?;
    let raw_image =
        fs::File::open(&raw_image_path).map_err(|error| LiveError::io(&raw_image_path, error))?;
    let normalized = crate::holo_rootfs_archive::normalize(raw_image, &image_tag)?;
    if normalized.image_id != observed_image_id {
        return Err(LiveError::Protocol(format!(
            "normalized Docker archive image {} does not match observed image {observed_image_id}",
            normalized.image_id
        )));
    }
    let image_bytes = u64::try_from(normalized.bytes.len()).unwrap_or(u64::MAX);
    let compressed =
        zstd::stream::encode_all(normalized.bytes.as_slice(), OCI_COMPRESSION_LEVEL)
            .map_err(|error| LiveError::Io(format!("compress Python OCI image: {error}")))?;
    let image_id = normalized.image_id;
    let metadata = BundleMetadata {
        schema_version: BUNDLE_SCHEMA_VERSION,
        provider: BUNDLE_PROVIDER.to_owned(),
        image_tag,
        image_id: Some(image_id.clone()),
        entry: source.entry.clone(),
        arch: normalize_arch(arch)?.to_owned(),
        image_uncompressed_bytes: image_bytes,
    };
    let bytes = encode_bundle(&metadata, &compressed)?;
    provenance.output = Some(BuildOutput {
        bundle_schema_version: BUNDLE_SCHEMA_VERSION,
        provider: BUNDLE_PROVIDER,
        layer_kappa: address_bytes(&bytes).to_string(),
        byte_length: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
        image_id,
        image_uncompressed_bytes: image_bytes,
    });
    Ok(CompiledRootfs { bytes, provenance })
}

fn docker_build_args(
    platform: &str,
    image_tag: &str,
    dockerfile: &str,
    source_date_epoch: &str,
    no_build_cache: bool,
) -> Vec<String> {
    let mut args = vec![
        "build".to_owned(),
        "--platform".to_owned(),
        platform.to_owned(),
        "--tag".to_owned(),
        image_tag.to_owned(),
        "--file".to_owned(),
        dockerfile.to_owned(),
        "--build-arg".to_owned(),
        source_date_epoch.to_owned(),
        "--provenance=false".to_owned(),
    ];
    if no_build_cache {
        args.push("--no-cache".to_owned());
    }
    args.push(".".to_owned());
    args
}

fn copy_image_file(image_tag: &str, source: &str, destination: &Path) -> Result<()> {
    let created = Command::new("docker")
        .args(["container", "create", "--", image_tag])
        .output()
        .map_err(|error| LiveError::Io(format!("create Docker builder container: {error}")))?;
    command_succeeded("Docker builder container create", &created)?;
    let container = String::from_utf8_lossy(&created.stdout).trim().to_owned();
    if container.is_empty() {
        return Err(LiveError::Protocol(
            "Docker did not return a builder container ID".to_owned(),
        ));
    }

    let copied = Command::new("docker")
        .args(["container", "cp"])
        .arg(format!("{container}:{source}"))
        .arg(destination)
        .output()
        .map_err(|error| LiveError::Io(format!("copy Docker builder output: {error}")));
    let removed = Command::new("docker")
        .args(["container", "rm", "--", &container])
        .output();
    match removed {
        Ok(removed) if !removed.status.success() => tracing::warn!(
            container,
            diagnostic = %diagnostic(&removed.stderr),
            "failed to remove temporary Docker builder container"
        ),
        Err(error) => tracing::warn!(
            container,
            %error,
            "failed to start temporary Docker builder container cleanup"
        ),
        Ok(_) => {}
    }
    let copied = copied?;
    command_succeeded("Docker builder output copy", &copied)
}

fn remove_image_tag(image_tag: &str) {
    match Command::new("docker")
        .args(["image", "rm", "--", image_tag])
        .output()
    {
        Ok(output) if !output.status.success() => tracing::warn!(
            image_tag,
            diagnostic = %diagnostic(&output.stderr),
            "failed to remove temporary Docker builder image"
        ),
        Err(error) => tracing::warn!(
            image_tag,
            %error,
            "failed to start temporary Docker builder image cleanup"
        ),
        Ok(_) => {}
    }
}

pub fn check_source(
    root: &Path,
    source: &PythonRootfsSource,
    arch: &str,
) -> Result<BuildProvenance> {
    validate_source(source, Some(arch))?;
    let inputs = resolve_inputs(root, source)?;
    build_provenance(source, &inputs, arch)
}

fn build_provenance(
    source: &PythonRootfsSource,
    inputs: &SourceInputs,
    arch: &str,
) -> Result<BuildProvenance> {
    let digest_pinned = digest_pinned_base(&source.base);
    Ok(BuildProvenance {
        profile: "rootfs",
        target_platform: docker_platform(arch)?.to_owned(),
        build_host: crate::holo_python_component::BuildHost {
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
        },
        compiler: BuildTool {
            name: "hologram-live",
            version: env!("CARGO_PKG_VERSION").to_owned(),
        },
        base_image: BaseImageProvenance {
            reference: source.base.clone(),
            digest_pinned,
            resolved_reference: digest_pinned.then(|| source.base.clone()),
            observed_image_id: None,
        },
        dependency_installer: BuildTool {
            name: "uv",
            version: UV_VERSION.to_owned(),
        },
        builder: DockerBuilder {
            name: "docker",
            archive_format: ROOTFS_REPRESENTATION,
            source_date_epoch: SOURCE_DATE_EPOCH,
            cache_disabled: false,
            client_version: None,
            server_version: None,
        },
        inputs: crate::holo_python_component::source_build_inputs(source, inputs)?,
        output: None,
        reproducibility: Reproducibility {
            reproducible: digest_pinned,
            blocker: (!digest_pinned).then_some(ROOTFS_MUTABLE_BASE_BLOCKER),
        },
    })
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
    if metadata.schema_version != BUNDLE_SCHEMA_VERSION || metadata.provider != BUNDLE_PROVIDER {
        return Err(LiveError::Capability(format!(
            "unsupported Python rootfs provider {} schema {}",
            metadata.provider, metadata.schema_version
        )));
    }
    validate_entry(&metadata.entry).map_err(|error| LiveError::InvalidHolo(error.to_string()))?;
    normalize_arch(&metadata.arch).map_err(|error| LiveError::InvalidHolo(error.to_string()))?;
    if !metadata.image_id.as_deref().is_some_and(valid_image_id) {
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

fn observed_docker_builder(cache_disabled: bool) -> Result<DockerBuilder> {
    Ok(DockerBuilder {
        name: "docker",
        archive_format: ROOTFS_REPRESENTATION,
        source_date_epoch: SOURCE_DATE_EPOCH,
        cache_disabled,
        client_version: Some(docker_version("Client")?),
        server_version: Some(docker_version("Server")?),
    })
}

fn resolve_base_image(base: &str) -> Result<String> {
    if digest_pinned_base(base) {
        return Ok(base.to_owned());
    }
    let output = Command::new("docker")
        .args(["buildx", "imagetools", "inspect", "--raw", "--", base])
        .output()
        .map_err(|error| {
            LiveError::Io(format!(
                "resolve Python OCI base image {base} through Docker: {error}"
            ))
        })?;
    command_succeeded(&format!("resolve Python OCI base image {base}"), &output)?;
    resolved_base_reference(base, &output.stdout)
}

fn resolved_base_reference(base: &str, manifest: &[u8]) -> Result<String> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct RegistryManifest {
        schema_version: u16,
    }

    if manifest.is_empty() {
        return Err(LiveError::Protocol(format!(
            "Docker returned an empty registry manifest for {base}"
        )));
    }
    let decoded: RegistryManifest = serde_json::from_slice(manifest).map_err(|error| {
        LiveError::Protocol(format!(
            "Docker returned an invalid registry manifest for {base}: {error}"
        ))
    })?;
    if decoded.schema_version != 2 {
        return Err(LiveError::Protocol(format!(
            "Docker returned unsupported registry manifest schema {} for {base}; expected 2",
            decoded.schema_version
        )));
    }
    let digest = crate::util::hex(&Sha256::digest(manifest));
    Ok(format!("{}@sha256:{digest}", base_repository(base)))
}

fn base_repository(base: &str) -> &str {
    let without_digest = base
        .split_once('@')
        .map_or(base, |(repository, _)| repository);
    let last_slash = without_digest.rfind('/');
    match without_digest.rfind(':') {
        Some(tag_separator) if last_slash.is_none_or(|slash| tag_separator > slash) => {
            &without_digest[..tag_separator]
        }
        _ => without_digest,
    }
}

fn docker_version(component: &str) -> Result<String> {
    let format = format!("{{{{.{component}.Version}}}}");
    let output = Command::new("docker")
        .args(["version", "--format", &format])
        .output()
        .map_err(|error| LiveError::Io(format!("query Docker {component} version: {error}")))?;
    command_succeeded(&format!("query Docker {component} version"), &output)?;
    let version = String::from_utf8(output.stdout)
        .map_err(|error| LiveError::Protocol(format!("decode Docker version: {error}")))?;
    let version = version.trim();
    if version.is_empty() {
        return Err(LiveError::Protocol(format!(
            "Docker returned an empty {component} version"
        )));
    }
    Ok(version.to_owned())
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

fn builder_dockerfile(base: &str) -> String {
    format!(
        "FROM {base} AS builder\n\
         ARG SOURCE_DATE_EPOCH=0\n\
         ENV PYTHONDONTWRITEBYTECODE=1 PYTHONUNBUFFERED=1 UV_COMPILE_BYTECODE=0\n\
         WORKDIR /app\n\
         RUN python -m pip install --no-cache-dir uv=={UV_VERSION}\n\
         COPY pyproject.toml uv.lock ./\n\
         COPY src ./src\n\
         RUN uv sync --locked --no-dev --no-editable --no-install-project\n\
         COPY .hologram/runner.py /hologram/runner.py\n\
         RUN find /app /hologram -exec touch -h -d '@0' {{}} + \\\n          && tar --sort=name --format=gnu --mtime='@0' --owner=0 --group=0 --numeric-owner \\\n            -cf /runtime.tar -C / app hologram \\\n          && touch -h -d '@0' /runtime.tar\n\
"
    )
}

fn runtime_dockerfile(base: &str, entry: &str) -> String {
    format!(
        "FROM {base}\n\
         ARG SOURCE_DATE_EPOCH=0\n\
         ENV PYTHONDONTWRITEBYTECODE=1 PYTHONUNBUFFERED=1 UV_COMPILE_BYTECODE=0\n\
         ADD runtime.tar /\n\
         WORKDIR /app\n\
         ENV PATH=\"/app/.venv/bin:$PATH\" PYTHONPATH=\"/app/src\" HOLOGRAM_PYTHON_ENTRY=\"{entry}\"\n\
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

pub(crate) fn validate_entry(entry: &str) -> Result<()> {
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
        || base.starts_with('-')
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

fn digest_pinned_base(base: &str) -> bool {
    base.rsplit_once("@sha256:").is_some_and(|(_, digest)| {
        digest.len() == 64
            && digest
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    })
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

pub(crate) fn resolve_inputs(root: &Path, source: &PythonRootfsSource) -> Result<SourceInputs> {
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
        let head = trimmed.chars().take(2_048).collect::<String>();
        let tail = trimmed.chars().rev().take(2_048).collect::<Vec<_>>();
        format!("{head}\n…\n{}", tail.into_iter().rev().collect::<String>())
    }
}

fn default_base() -> String {
    "python:3.12-slim".to_owned()
}

fn is_default_base(base: &str) -> bool {
    base == "python:3.12-slim"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metadata() -> BundleMetadata {
        BundleMetadata {
            schema_version: BUNDLE_SCHEMA_VERSION,
            provider: BUNDLE_PROVIDER.to_owned(),
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
    fn base_digest_pin_requires_a_lowercase_sha256() {
        assert!(digest_pinned_base(&format!(
            "python@sha256:{}",
            "a".repeat(64)
        )));
        assert!(!digest_pinned_base("python:3.12-slim"));
        assert!(!digest_pinned_base(&format!(
            "python@sha256:{}",
            "A".repeat(64)
        )));
        assert!(!digest_pinned_base("python@sha256:abc"));
    }

    #[test]
    fn registry_manifest_digest_qualifies_the_requested_repository() {
        let manifest = br#"{"schemaVersion":2,"mediaType":"application/vnd.oci.image.index.v1+json","manifests":[]}"#;
        let digest = crate::util::hex(&Sha256::digest(manifest));
        assert_eq!(
            resolved_base_reference("python:3.12-slim", manifest).expect("resolve tag"),
            format!("python@sha256:{digest}")
        );
        assert_eq!(
            resolved_base_reference("registry.example:5000/team/python:stable", manifest)
                .expect("resolve registry tag"),
            format!("registry.example:5000/team/python@sha256:{digest}")
        );
    }

    #[test]
    fn digest_pinned_base_resolution_is_offline_and_preserves_the_reference() {
        let pinned = format!("registry.example/team/python@sha256:{}", "a".repeat(64));
        assert_eq!(resolve_base_image(&pinned).expect("resolve pin"), pinned);
    }

    #[test]
    fn planned_provenance_reports_a_digest_pinned_base_as_resolved() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let pinned = format!("python@sha256:{}", "a".repeat(64));
        let source = PythonRootfsSource {
            project: PathBuf::from("examples/python-numpy-pandas"),
            entry: "numpy_pandas_holo:main".to_owned(),
            lock: PathBuf::from("uv.lock"),
            profile: PythonProfile::Rootfs,
            base: pinned.clone(),
        };
        let inputs = resolve_inputs(root, &source).expect("resolve inputs");
        let provenance = build_provenance(&source, &inputs, "arm64").expect("provenance");
        assert_eq!(
            provenance.base_image.resolved_reference.as_deref(),
            Some(pinned.as_str())
        );
        assert_eq!(
            provenance.reproducibility,
            Reproducibility {
                reproducible: true,
                blocker: None,
            }
        );
    }

    #[test]
    fn registry_manifest_resolution_rejects_invalid_or_unsupported_manifests() {
        let empty = resolved_base_reference("python:latest", b"").expect_err("empty");
        assert_eq!(empty.code(), "LIVE_PROTOCOL_ERROR");
        let invalid = resolved_base_reference("python:latest", b"not json").expect_err("invalid");
        assert_eq!(invalid.code(), "LIVE_PROTOCOL_ERROR");
        let unsupported = resolved_base_reference("python:latest", br#"{"schemaVersion":1}"#)
            .expect_err("unsupported schema");
        assert_eq!(unsupported.code(), "LIVE_PROTOCOL_ERROR");
    }

    #[test]
    fn dockerfile_uses_the_resolved_base_reference() {
        let resolved = format!("python@sha256:{}", "a".repeat(64));
        let builder = builder_dockerfile(&resolved);
        assert!(builder.starts_with(&format!("FROM {resolved} AS builder\n")));
        assert_eq!(builder.matches(&format!("FROM {resolved}")).count(), 1);
        assert!(builder.contains("--no-install-project"));
        assert!(builder.contains("touch -h -d '@0'"));
        assert!(builder.contains("tar --sort=name --format=gnu"));

        let runtime = runtime_dockerfile(&resolved, "example:main");
        assert!(runtime.starts_with(&format!("FROM {resolved}\n")));
        assert!(runtime.contains("ADD runtime.tar /"));
        assert!(runtime.contains("PYTHONPATH=\"/app/src\""));
        assert!(!format!("{builder}{runtime}").contains("python:3.12-slim"));
    }

    #[test]
    fn docker_build_cache_is_disabled_only_when_requested() {
        let cached = docker_build_args(
            "linux/arm64",
            "hologram-python-test:local",
            "Dockerfile",
            "SOURCE_DATE_EPOCH=0",
            false,
        );
        assert!(!cached.iter().any(|argument| argument == "--no-cache"));

        let uncached = docker_build_args(
            "linux/arm64",
            "hologram-python-test:local",
            "Dockerfile",
            "SOURCE_DATE_EPOCH=0",
            true,
        );
        assert_eq!(uncached[uncached.len() - 2], "--no-cache");
        assert_eq!(uncached.last().map(String::as_str), Some("."));
    }

    #[test]
    fn long_diagnostics_preserve_the_command_context_and_final_error() {
        let input = format!("context:{}:failure", "x".repeat(5_000));
        let output = diagnostic(input.as_bytes());
        assert!(output.starts_with("context:"));
        assert!(output.ends_with(":failure"));
        assert!(output.contains("\n…\n"));
        assert!(output.chars().count() <= 4_099);
    }

    #[test]
    fn base_reference_cannot_be_parsed_as_a_docker_option() {
        assert!(validate_base("-q").is_err());
    }

    #[test]
    fn rejects_truncated_bundle() {
        let error = decode_bundle(b"HOLOPYR2\xff\xff\xff\xff").expect_err("invalid");
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
