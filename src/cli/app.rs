use super::Cli;
use clap::{Args, Subcommand, ValueEnum};
use hologram_live::compile::{
    validate_compile_manifest, CompileLayer, CompileLayerKind, CompileManifest, CompileSource,
};
use hologram_live::error::{LiveError, Result};
use hologram_live::holo_python::{PythonProfile, PythonRootfsSource};
use hologram_live::util::atomic_write;
use serde::Serialize;
use std::io::{BufRead, IsTerminal, Write};
use std::path::PathBuf;

#[derive(Debug, Clone, Args)]
pub struct AppArgs {
    #[command(subcommand)]
    command: AppCommand,
}

#[derive(Debug, Clone, Subcommand)]
enum AppCommand {
    /// Generate a validated hologram.json application manifest.
    Init(InitArgs),
}

#[derive(Debug, Clone, Args)]
struct InitArgs {
    /// Directory that will contain hologram.json.
    #[arg(default_value = ".")]
    directory: PathBuf,
    /// Source-language application template.
    #[arg(long, value_enum)]
    template: Option<TemplateArg>,
    /// Kind of the first layer.
    #[arg(long, value_enum)]
    kind: Option<LayerKindArg>,
    /// Path to the first layer, relative to hologram.json.
    #[arg(long)]
    path: Option<PathBuf>,
    /// Entrypoint for a Wasm, tensor, or rootfs layer.
    #[arg(long)]
    entry: Option<String>,
    /// Architecture required by a rootfs layer.
    #[arg(long)]
    arch: Option<String>,
    /// Surface required by a view layer.
    #[arg(long)]
    surface: Option<String>,
    /// Python execution profile.
    #[arg(long, value_enum)]
    profile: Option<PythonProfileArg>,
    /// Python project path, relative to hologram.json.
    #[arg(long)]
    project: Option<PathBuf>,
    /// Python lock file, relative to the project.
    #[arg(long)]
    lock: Option<PathBuf>,
    /// OCI base image used by a Python rootfs build.
    #[arg(long)]
    base: Option<String>,
    /// Zero-based primary layer position.
    #[arg(long, conflicts_with = "no_primary")]
    primary: Option<u32>,
    /// Generate a manifest without an exit-bearing primary layer.
    #[arg(long)]
    no_primary: bool,
    /// Capability document path, relative to hologram.json.
    #[arg(long)]
    capabilities: Option<PathBuf>,
    /// Accept defaults for omitted first-layer fields without prompting.
    #[arg(long)]
    yes: bool,
    /// Replace an existing hologram.json.
    #[arg(long)]
    force: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum LayerKindArg {
    Wasm,
    Tensor,
    Rootfs,
    View,
}

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
enum TemplateArg {
    Python,
}

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
enum PythonProfileArg {
    Rootfs,
}

#[derive(Debug, Serialize)]
struct InitReport {
    manifest: PathBuf,
    layer_count: usize,
    primary: Option<u32>,
    compile_command: String,
    thin_compile_command: String,
    run_command: Option<String>,
}

pub async fn run(cli: Cli, args: AppArgs) -> Result<()> {
    match args.command {
        AppCommand::Init(args) => {
            let interactive = std::io::stdin().is_terminal()
                && std::io::stderr().is_terminal()
                && !args.yes
                && !has_layer_flags(&args);
            let report = initialize(
                args,
                interactive,
                &mut std::io::stdin().lock(),
                &mut std::io::stderr().lock(),
            )?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!("initialized {}", report.manifest.display());
                println!("compile: {}", report.compile_command);
                println!("compile thin: {}", report.thin_compile_command);
                if let Some(command) = report.run_command {
                    println!("run: {command}");
                }
            }
            Ok(())
        }
    }
}

fn has_layer_flags(args: &InitArgs) -> bool {
    args.kind.is_some()
        || args.template.is_some()
        || args.path.is_some()
        || args.entry.is_some()
        || args.arch.is_some()
        || args.surface.is_some()
        || args.profile.is_some()
        || args.project.is_some()
        || args.lock.is_some()
        || args.base.is_some()
}

fn initialize<R: BufRead, W: Write>(
    args: InitArgs,
    interactive: bool,
    input: &mut R,
    output: &mut W,
) -> Result<InitReport> {
    let manifest_path = args.directory.join("hologram.json");
    if manifest_path.exists() && !args.force {
        return Err(LiveError::Conflict(format!(
            "{} already exists; pass --force to replace it",
            manifest_path.display()
        )));
    }

    let python = args.template == Some(TemplateArg::Python);
    let mut layers = if python {
        vec![python_layer_from_args(&args)?]
    } else if interactive {
        interactive_layers(input, output)?
    } else {
        vec![layer_from_args(&args)?]
    };
    if layers.is_empty() {
        return Err(LiveError::Config(
            "an application manifest requires at least one layer".to_owned(),
        ));
    }

    let primary = if args.no_primary {
        None
    } else if let Some(primary) = args.primary {
        Some(primary)
    } else if interactive {
        prompt_primary(input, output, &layers)?
    } else {
        default_primary(&layers)
    };
    let requires = if let Some(path) = args.capabilities {
        Some(path)
    } else if interactive {
        prompt_optional_path(input, output, "Capability file (optional)")?
    } else {
        None
    };

    let specification = CompileManifest {
        schema_version: if python { 2 } else { 1 },
        primary,
        requires,
        layers: std::mem::take(&mut layers),
    };
    validate_compile_manifest(&specification)?;
    let mut encoded = serde_json::to_vec_pretty(&specification)?;
    encoded.push(b'\n');
    atomic_write(&manifest_path, &encoded)?;

    let archive = args.directory.join("application.holo");
    let compile_command = format!(
        "hologram compile {} --output {}",
        display_path(&manifest_path),
        display_path(&archive)
    );
    let thin_archive = args.directory.join("application.thin.holo");
    let thin_compile_command = format!(
        "hologram compile {} --thin --output {}",
        display_path(&manifest_path),
        display_path(&thin_archive)
    );
    let run_command = primary.map(|_| format!("hologram run {}", display_path(&archive)));
    Ok(InitReport {
        manifest: manifest_path,
        layer_count: specification.layers.len(),
        primary,
        compile_command,
        thin_compile_command,
        run_command,
    })
}

fn layer_from_args(args: &InitArgs) -> Result<CompileLayer> {
    if args.template.is_some() {
        return Err(LiveError::Config(
            "source templates cannot be combined with --kind or --path".to_owned(),
        ));
    }
    let kind = args
        .kind
        .or(args.yes.then_some(LayerKindArg::Wasm))
        .ok_or_else(|| {
            LiveError::Config(
                "non-interactive app init requires --kind and --path, or --yes for defaults"
                    .to_owned(),
            )
        })?;
    let path = args
        .path
        .clone()
        .or_else(|| args.yes.then(|| default_path(kind)))
        .ok_or_else(|| {
            LiveError::Config(
                "non-interactive app init requires --kind and --path, or --yes for defaults"
                    .to_owned(),
            )
        })?;
    Ok(CompileLayer {
        kind: kind.into(),
        path: Some(path),
        source: None,
        entry: args.entry.clone(),
        arch: args.arch.clone(),
        surface: args.surface.clone(),
    })
}

fn python_layer_from_args(args: &InitArgs) -> Result<CompileLayer> {
    if args.kind.is_some() || args.path.is_some() || args.surface.is_some() || args.yes {
        return Err(LiveError::Config(
            "--template python cannot be combined with --kind, --path, --surface, or --yes"
                .to_owned(),
        ));
    }
    if args.profile.unwrap_or(PythonProfileArg::Rootfs) != PythonProfileArg::Rootfs {
        return Err(LiveError::Config(
            "Python applications currently support only --profile rootfs".to_owned(),
        ));
    }
    let entry = args.entry.clone().ok_or_else(|| {
        LiveError::Config("--template python requires --entry module:function".to_owned())
    })?;
    Ok(CompileLayer {
        kind: CompileLayerKind::Rootfs,
        path: None,
        source: Some(CompileSource::Python(PythonRootfsSource {
            project: args.project.clone().unwrap_or_else(|| ".".into()),
            entry,
            lock: args.lock.clone().unwrap_or_else(|| "uv.lock".into()),
            profile: PythonProfile::Rootfs,
            base: args
                .base
                .clone()
                .unwrap_or_else(|| "python:3.12-slim".to_owned()),
        })),
        entry: None,
        arch: Some(args.arch.clone().unwrap_or_else(default_arch)),
        surface: None,
    })
}

fn interactive_layers<R: BufRead, W: Write>(
    input: &mut R,
    output: &mut W,
) -> Result<Vec<CompileLayer>> {
    let mut layers = Vec::new();
    loop {
        let kind = prompt_kind(input, output)?;
        let path = PathBuf::from(prompt(
            input,
            output,
            "Layer path",
            Some(&default_path(kind).to_string_lossy()),
        )?);
        let (entry, arch, surface) = match kind {
            LayerKindArg::Wasm => (
                Some(prompt(input, output, "Entrypoint", Some("_start"))?),
                None,
                None,
            ),
            LayerKindArg::Tensor => (
                Some(prompt(
                    input,
                    output,
                    "Session entrypoint",
                    Some("session"),
                )?),
                None,
                None,
            ),
            LayerKindArg::Rootfs => (
                Some(prompt(input, output, "Boot entrypoint", Some("boot"))?),
                Some(prompt_required(input, output, "Architecture")?),
                None,
            ),
            LayerKindArg::View => (
                None,
                None,
                Some(prompt(input, output, "Surface", Some("portable"))?),
            ),
        };
        layers.push(CompileLayer {
            kind: kind.into(),
            path: Some(path),
            source: None,
            entry,
            arch,
            surface,
        });
        if !prompt_yes_no(input, output, "Add another layer", false)? {
            break;
        }
    }
    Ok(layers)
}

fn prompt_kind<R: BufRead, W: Write>(input: &mut R, output: &mut W) -> Result<LayerKindArg> {
    loop {
        let value = prompt(
            input,
            output,
            "Layer kind (wasm/tensor/rootfs/view)",
            Some("wasm"),
        )?;
        let kind = match value.to_ascii_lowercase().as_str() {
            "wasm" => Some(LayerKindArg::Wasm),
            "tensor" => Some(LayerKindArg::Tensor),
            "rootfs" => Some(LayerKindArg::Rootfs),
            "view" => Some(LayerKindArg::View),
            _ => None,
        };
        if let Some(kind) = kind {
            return Ok(kind);
        }
        writeln!(output, "Choose wasm, tensor, rootfs, or view.").map_err(prompt_write_error)?;
    }
}

fn prompt_primary<R: BufRead, W: Write>(
    input: &mut R,
    output: &mut W,
    layers: &[CompileLayer],
) -> Result<Option<u32>> {
    let default = default_primary(layers);
    let default_text = default.map_or_else(|| "none".to_owned(), |value| value.to_string());
    loop {
        let value = prompt(input, output, "Primary layer index", Some(&default_text))?;
        if value.eq_ignore_ascii_case("none") {
            return Ok(None);
        }
        if let Ok(index) = value.parse::<u32>() {
            return Ok(Some(index));
        }
        writeln!(output, "Enter a zero-based layer index or none.").map_err(prompt_write_error)?;
    }
}

fn prompt_optional_path<R: BufRead, W: Write>(
    input: &mut R,
    output: &mut W,
    label: &str,
) -> Result<Option<PathBuf>> {
    let value = prompt(input, output, label, Some(""))?;
    Ok((!value.is_empty()).then(|| PathBuf::from(value)))
}

fn prompt_required<R: BufRead, W: Write>(
    input: &mut R,
    output: &mut W,
    label: &str,
) -> Result<String> {
    loop {
        let value = prompt(input, output, label, None)?;
        if !value.is_empty() {
            return Ok(value);
        }
        writeln!(output, "{label} is required.").map_err(prompt_write_error)?;
    }
}

fn prompt_yes_no<R: BufRead, W: Write>(
    input: &mut R,
    output: &mut W,
    label: &str,
    default: bool,
) -> Result<bool> {
    loop {
        let default_value = if default { "yes" } else { "no" };
        let value = prompt(input, output, label, Some(default_value))?;
        match value.to_ascii_lowercase().as_str() {
            "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            _ => writeln!(output, "Enter yes or no.").map_err(prompt_write_error)?,
        }
    }
}

fn prompt<R: BufRead, W: Write>(
    input: &mut R,
    output: &mut W,
    label: &str,
    default: Option<&str>,
) -> Result<String> {
    match default {
        Some(default) if !default.is_empty() => write!(output, "{label} [{default}]: "),
        Some(_) | None => write!(output, "{label}: "),
    }
    .map_err(prompt_write_error)?;
    output.flush().map_err(prompt_write_error)?;
    let mut value = String::new();
    let read = input.read_line(&mut value).map_err(prompt_read_error)?;
    if read == 0 {
        return default.map(str::to_owned).ok_or_else(|| {
            LiveError::Config(format!("interactive input ended while reading {label}"))
        });
    }
    let value = value.trim();
    Ok(if value.is_empty() {
        default.unwrap_or_default().to_owned()
    } else {
        value.to_owned()
    })
}

fn default_primary(layers: &[CompileLayer]) -> Option<u32> {
    layers
        .iter()
        .position(|layer| {
            matches!(
                layer.kind,
                CompileLayerKind::Wasm | CompileLayerKind::Rootfs
            )
        })
        .and_then(|position| u32::try_from(position).ok())
}

fn default_path(kind: LayerKindArg) -> PathBuf {
    match kind {
        LayerKindArg::Wasm => "app.wasm",
        LayerKindArg::Tensor => "model.bin",
        LayerKindArg::Rootfs => "rootfs.img",
        LayerKindArg::View => "index.html",
    }
    .into()
}

fn default_arch() -> String {
    match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "x86_64",
        arch => arch,
    }
    .to_owned()
}

fn display_path(path: &std::path::Path) -> String {
    path.to_string_lossy().into_owned()
}

fn prompt_read_error(error: std::io::Error) -> LiveError {
    LiveError::Io(format!("read interactive app manifest input: {error}"))
}

fn prompt_write_error(error: std::io::Error) -> LiveError {
    LiveError::Io(format!("write interactive app manifest prompt: {error}"))
}

impl From<LayerKindArg> for CompileLayerKind {
    fn from(value: LayerKindArg) -> Self {
        match value {
            LayerKindArg::Wasm => Self::Wasm,
            LayerKindArg::Tensor => Self::Tensor,
            LayerKindArg::Rootfs => Self::Rootfs,
            LayerKindArg::View => Self::View,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn args(directory: PathBuf) -> InitArgs {
        InitArgs {
            directory,
            template: None,
            kind: None,
            path: None,
            entry: None,
            arch: None,
            surface: None,
            profile: None,
            project: None,
            lock: None,
            base: None,
            primary: None,
            no_primary: false,
            capabilities: None,
            yes: false,
            force: false,
        }
    }

    #[test]
    fn interactive_flow_generates_multiple_valid_layers() {
        let directory = tempfile::tempdir().expect("tempdir");
        let input =
            b"wasm\napp.wasm\nholo_run\ny\nview\nindex.html\nportable\nn\n0\ncapabilities.json\n";
        let report = initialize(
            args(directory.path().to_path_buf()),
            true,
            &mut Cursor::new(input),
            &mut Vec::new(),
        )
        .expect("initialize");
        let bytes = std::fs::read(&report.manifest).expect("manifest");
        let manifest: CompileManifest = serde_json::from_slice(&bytes).expect("parse");
        assert_eq!(manifest.layers.len(), 2);
        assert_eq!(manifest.primary, Some(0));
        assert_eq!(
            manifest.requires.as_deref(),
            Some(std::path::Path::new("capabilities.json"))
        );
        assert!(matches!(manifest.layers[1].kind, CompileLayerKind::View));
    }

    #[test]
    fn yes_uses_a_minimal_wasm_manifest() {
        let directory = tempfile::tempdir().expect("tempdir");
        let mut options = args(directory.path().to_path_buf());
        options.yes = true;
        initialize(options, false, &mut Cursor::new([]), &mut Vec::new()).expect("initialize");
        let bytes = std::fs::read(directory.path().join("hologram.json")).expect("manifest");
        let manifest: CompileManifest = serde_json::from_slice(&bytes).expect("parse");
        assert_eq!(manifest.primary, Some(0));
        assert_eq!(
            manifest.layers[0].path.as_deref(),
            Some(std::path::Path::new("app.wasm"))
        );
    }

    #[test]
    fn non_interactive_mode_requires_flags_or_yes() {
        let directory = tempfile::tempdir().expect("tempdir");
        let error = initialize(
            args(directory.path().to_path_buf()),
            false,
            &mut Cursor::new([]),
            &mut Vec::new(),
        )
        .expect_err("missing flags");
        assert_eq!(error.code(), "LIVE_CONFIG_INVALID");
    }

    #[test]
    fn existing_manifest_requires_force() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("hologram.json");
        std::fs::write(&path, "original").expect("original");
        let mut options = args(directory.path().to_path_buf());
        options.yes = true;
        let error = initialize(options, false, &mut Cursor::new([]), &mut Vec::new())
            .expect_err("must refuse");
        assert_eq!(error.code(), "LIVE_CONFLICT");
        assert_eq!(std::fs::read_to_string(path).expect("read"), "original");
    }

    #[test]
    fn force_replaces_an_existing_manifest() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("hologram.json");
        std::fs::write(&path, "original").expect("original");
        let mut options = args(directory.path().to_path_buf());
        options.yes = true;
        options.force = true;
        initialize(options, false, &mut Cursor::new([]), &mut Vec::new()).expect("replace");
        let manifest: CompileManifest =
            serde_json::from_slice(&std::fs::read(path).expect("read")).expect("parse");
        assert_eq!(manifest.layers.len(), 1);
    }

    #[test]
    fn kind_specific_fields_are_validated_non_interactively() {
        let directory = tempfile::tempdir().expect("tempdir");
        let mut options = args(directory.path().to_path_buf());
        options.kind = Some(LayerKindArg::View);
        options.path = Some("index.html".into());
        let error = initialize(options, false, &mut Cursor::new([]), &mut Vec::new())
            .expect_err("missing surface");
        assert_eq!(error.code(), "LIVE_CONFIG_INVALID");
        assert!(error.to_string().contains("require surface"), "{error}");
    }

    #[test]
    fn python_template_generates_a_schema_two_rootfs_source() {
        let directory = tempfile::tempdir().expect("tempdir");
        let mut options = args(directory.path().to_path_buf());
        options.template = Some(TemplateArg::Python);
        options.entry = Some("numpy_pandas_holo:main".to_owned());
        options.arch = Some("arm64".to_owned());
        initialize(options, false, &mut Cursor::new([]), &mut Vec::new()).expect("initialize");

        let manifest: CompileManifest = serde_json::from_slice(
            &std::fs::read(directory.path().join("hologram.json")).expect("manifest"),
        )
        .expect("parse");
        assert_eq!(manifest.schema_version, 2);
        assert_eq!(manifest.primary, Some(0));
        assert!(matches!(
            manifest.layers[0].source,
            Some(CompileSource::Python(_))
        ));
    }
}
