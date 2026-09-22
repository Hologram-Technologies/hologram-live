//! `hologram oci …`: the registry's operator commands.
//!
//! Declared so the reference's command names reach them (`registry
//! garbage-collect` becomes `hologram oci garbage-collect`). `verify` is
//! built; the others are built in the operator-commands work (plan, P8) and
//! until then say so.

use super::Cli;
use clap::{Args, Subcommand};
use hologram_live::error::{LiveError, Result};
use hologram_live::oci_store::{OciStore, OciStoreError, OpenOptions, VerifyReport};
use std::io::Write as _;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Args)]
pub struct OciArgs {
    #[command(subcommand)]
    command: OciCommand,
}

#[derive(Debug, Clone, Subcommand)]
enum OciCommand {
    /// Remove blobs no manifest references, as the reference's garbage-collect.
    GarbageCollect {
        /// Report what would be removed, and remove nothing.
        #[arg(long, short = 'd')]
        dry_run: bool,
        /// Also remove manifests no tag points at.
        #[arg(long, short = 'm')]
        delete_untagged: bool,
        /// Print nothing but errors.
        #[arg(long, short = 'q')]
        quiet: bool,
        /// The reference registry's configuration file.
        #[arg(long)]
        registry_config: Option<PathBuf>,
    },
    /// Re-hash every blob with the algorithm of its own address, and name any
    /// that does not match, with the repositories and tags that reach it.
    ///
    /// Reports and changes nothing: no blob is deleted or moved. Opening the
    /// volume does what a start does (interrupted uploads are resumed or
    /// dropped). Exits 1 when a blob is damaged, and only then. The registry must be stopped:
    /// verify beside a running server, through its administration socket, is
    /// not built yet.
    Verify {
        /// The registry's configuration file (or `HOLOGRAM_REGISTRY_CONFIG`, as
        /// for `serve`); else `REGISTRY_CONFIGURATION_PATH`, else the image's
        /// `/etc/distribution/config.yml`.
        #[arg(long, env = "HOLOGRAM_REGISTRY_CONFIG")]
        registry_config: Option<PathBuf>,
    },
    /// Copy a Docker Registry volume into this registry's layout.
    Import {
        /// The reference's volume (`/var/lib/registry`).
        source: PathBuf,
        /// A new, empty directory for this registry's volume.
        #[arg(long)]
        into: PathBuf,
    },
}

pub async fn run(cli: Cli, args: OciArgs) -> Result<()> {
    match args.command {
        OciCommand::Verify { registry_config } => verify(&cli, registry_config).await,
        OciCommand::GarbageCollect { .. } => not_built("garbage-collect"),
        OciCommand::Import { .. } => not_built("import"),
    }
}

fn not_built(name: &str) -> Result<()> {
    Err(LiveError::Capability(format!(
        "hologram oci {name} is not built yet (plan P8)"
    )))
}

/// The configuration file operator commands read, as `registry serve` finds it.
fn registry_file(given: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(file) = given {
        return Ok(file);
    }
    if let Some(file) = std::env::var_os("REGISTRY_CONFIGURATION_PATH") {
        return Ok(PathBuf::from(file));
    }
    let image = PathBuf::from("/etc/distribution/config.yml");
    if image.is_file() {
        return Ok(image);
    }
    Err(LiveError::Config(
        "give the registry's configuration file: --registry-config <file>, or REGISTRY_CONFIGURATION_PATH"
            .to_owned(),
    ))
}

/// The registry's volume: `storage.filesystem.rootdirectory`, as `serve` reads it.
fn volume(file: &Path) -> Result<PathBuf> {
    let settings = hologram_live::registry_compat::load(Some(file), std::env::vars())?;
    Ok(PathBuf::from(
        settings
            .get("storage.filesystem.rootdirectory")
            .unwrap_or("/var/lib/registry")
            .trim(),
    ))
}

async fn verify(cli: &Cli, registry_config: Option<PathBuf>) -> Result<()> {
    let root = volume(&registry_file(registry_config)?)?;
    let json = cli.json;
    let report = tokio::task::spawn_blocking(move || -> Result<VerifyReport> {
        // No age: verify must never expire an upload session the server
        // would have kept.
        let options = OpenOptions {
            create: false,
            upload_max_age: std::time::Duration::MAX,
        };
        // Exit 1 is kept for damage: a locked volume is a missing capability
        // (5), a volume that cannot be read a configuration fault (2).
        let store = OciStore::open(&root, options).map_err(|error| match error {
            OciStoreError::Locked => LiveError::Capability(format!(
                "the registry is running on {}: stop it first (verify beside a running server is not built yet)",
                root.display()
            )),
            other => LiveError::Config(format!("registry volume {}: {other}", root.display())),
        })?;
        let terminal = std::io::IsTerminal::is_terminal(&std::io::stderr());
        let mut last = std::time::Instant::now();
        store
            .verify(&mut |checked, total| {
                if terminal && (checked == total || last.elapsed().as_secs() >= 1) {
                    eprint!("\rchecked {checked} of {total} blobs");
                    if checked == total {
                        eprintln!();
                    }
                    last = std::time::Instant::now();
                }
            })
            .map_err(|error| LiveError::Config(format!("verify: {error}")))
    })
    .await
    .map_err(|error| LiveError::Config(format!("join verify: {error}")))??;
    print_report(&report, json)?;
    if !report.damaged.is_empty() {
        // Damage is the answer, not a failure of the command: the report is
        // out, and the exit code is for the operator's own automation.
        std::process::exit(1);
    }
    Ok(())
}

fn print_report(report: &VerifyReport, json: bool) -> Result<()> {
    let mut out = std::io::stdout().lock();
    let written = print_to(&mut out, report, json).and_then(|()| out.flush());
    written.map_err(|error| LiveError::Config(format!("print the report: {error}")))
}

fn print_to(
    out: &mut impl std::io::Write,
    report: &VerifyReport,
    json: bool,
) -> std::io::Result<()> {
    let written = if json {
        serde_json::to_writer_pretty(&mut *out, report)
            .map_err(std::io::Error::other)
            .and_then(|()| writeln!(out))
    } else if report.damaged.is_empty() {
        writeln!(
            out,
            "{} blobs, {} bytes: every blob matches its digest",
            report.checked, report.bytes
        )
    } else {
        let mut result = writeln!(
            out,
            "{} of {} blobs damaged ({} bytes read); nothing was changed. Push the tags below again, or restore the blobs:",
            report.damaged.len(),
            report.checked,
            report.bytes
        );
        for damaged in &report.damaged {
            result =
                result.and_then(|()| writeln!(out, "  {} ({})", damaged.digest, damaged.reason));
            if damaged.repositories.is_empty() {
                result = result.and_then(|()| {
                    writeln!(
                        out,
                        "    no repository links it (it may be one of the store's own records)"
                    )
                });
            }
            for reach in &damaged.repositories {
                let tags = if reach.tags.is_empty() {
                    "no tag reaches it".to_owned()
                } else {
                    reach.tags.join(", ")
                };
                result = result.and_then(|()| writeln!(out, "    {}: {tags}", reach.name));
            }
        }
        result
    };
    written
}
