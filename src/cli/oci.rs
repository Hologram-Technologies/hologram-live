//! `hologram oci …`: the registry's operator commands.
//!
//! Declared so the reference's command names reach them (`registry
//! garbage-collect` becomes `hologram oci garbage-collect`).

use super::Cli;
use clap::{Args, Subcommand};
use hologram_live::error::{LiveError, Result};
use hologram_live::oci_store::{
    GcOptions, ImportEvent, ImportReport, OciStore, OciStoreError, OpenOptions, VerifyReport,
};
use std::io::Write as _;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Args)]
pub struct OciArgs {
    #[command(subcommand)]
    command: OciCommand,
}

#[derive(Debug, Clone, Subcommand)]
enum OciCommand {
    /// Remove blobs no manifest references, as the reference's garbage-collect,
    /// printing what it marks and removes in the reference's words.
    ///
    /// Only objects the registry itself stored are ever removed. A manifest
    /// that cannot be read stops the run before anything is removed. The
    /// registry must be stopped.
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
        #[arg(long, env = "HOLOGRAM_REGISTRY_CONFIG")]
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
    ///
    /// Every blob is hashed on the way in, and every manifest is checked as a
    /// push of it would be. The source is only read. What is already here is
    /// skipped, so a second run adds nothing and an interrupted run finishes
    /// when run again. Exits 1 when anything the source serves did not come
    /// over, and names it; blobs the source links but no longer holds (its
    /// garbage collection leaves those links) are reported and do not fail.
    Import {
        /// The reference's volume (`/var/lib/registry`).
        source: PathBuf,
        /// This registry's volume: a new or empty directory, or one an earlier
        /// import wrote. The registry must not be running on it.
        #[arg(long)]
        into: PathBuf,
    },
}

pub async fn run(cli: Cli, args: OciArgs) -> Result<()> {
    match args.command {
        OciCommand::Verify { registry_config } => verify(&cli, registry_config).await,
        OciCommand::GarbageCollect {
            dry_run,
            delete_untagged,
            quiet,
            registry_config,
        } => {
            let options = GcOptions {
                dry_run,
                delete_untagged,
            };
            garbage_collect(options, quiet, registry_config).await
        }
        OciCommand::Import { source, into } => import(&cli, source, into).await,
    }
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

/// Open the volume of a stopped registry, never expiring an upload session.
fn open_stopped(root: &Path, command: &str) -> Result<OciStore> {
    let options = OpenOptions {
        create: false,
        upload_max_age: std::time::Duration::MAX,
    };
    OciStore::open(root, options).map_err(|error| match error {
        OciStoreError::Locked => LiveError::Capability(format!(
            "the registry is running on {}: stop it first ({command} beside a running server is not built yet)",
            root.display()
        )),
        other => LiveError::Config(format!("registry volume {}: {other}", root.display())),
    })
}

async fn garbage_collect(
    options: GcOptions,
    quiet: bool,
    registry_config: Option<PathBuf>,
) -> Result<()> {
    let root = volume(&registry_file(registry_config)?)?;
    tokio::task::spawn_blocking(move || -> Result<()> {
        // A volume nothing was ever pushed to: the reference's empty answer,
        // and nothing created.
        // A path that does not exist is a mistake, not an empty registry.
        let empty = std::fs::read_dir(&root)
            .map_err(|error| {
                LiveError::Config(format!("registry volume {}: {error}", root.display()))
            })?
            .next()
            .is_none();
        if empty {
            if !quiet {
                println!("\n0 blobs marked, 0 blobs and 0 manifests eligible for deletion");
            }
            return Ok(());
        }
        let store = open_stopped(&root, "garbage-collect")?;
        let mut out = std::io::stdout().lock();
        let mut failed = None;
        store
            .collect(options, &mut |line| {
                if !quiet && failed.is_none() {
                    if let Err(error) = writeln!(out, "{line}") {
                        failed = Some(error);
                    }
                }
            })
            .map_err(|error| LiveError::Config(format!("garbage-collect: {error}")))?;
        match failed {
            Some(error) => Err(LiveError::Config(format!("print: {error}"))),
            None => out
                .flush()
                .map_err(|error| LiveError::Config(format!("print: {error}"))),
        }
    })
    .await
    .map_err(|error| LiveError::Config(format!("join garbage-collect: {error}")))?
}

async fn import(cli: &Cli, source: PathBuf, into: PathBuf) -> Result<()> {
    let json = cli.json;
    let report = tokio::task::spawn_blocking(move || -> Result<ImportReport> {
        let store = open_for_import(&source, &into)?;
        let terminal = std::io::IsTerminal::is_terminal(&std::io::stderr());
        store
            .import(
                &source,
                &hologram_live::modules::oci::import_plan,
                &mut |event| match event {
                    ImportEvent::Repository { name, index, total } if terminal => {
                        eprintln!("[{}/{total}] {name}", index + 1);
                    }
                    ImportEvent::Problem(problem) if terminal => {
                        eprintln!("  {}: {}", problem.kind.as_str(), problem.detail);
                    }
                    _ => {}
                },
            )
            .map_err(|error| match error {
                OciStoreError::Layout(message) => LiveError::Config(message),
                other => LiveError::Config(format!("import: {other}")),
            })
    })
    .await
    .map_err(|error| LiveError::Config(format!("join import: {error}")))??;
    let mut out = std::io::stdout().lock();
    let written = if json {
        serde_json::to_writer_pretty(&mut out, &report)
            .map_err(std::io::Error::other)
            .and_then(|()| writeln!(out))
    } else {
        print_import(&mut out, &report)
    };
    written
        .and_then(|()| out.flush())
        .map_err(|error| LiveError::Config(format!("print the report: {error}")))?;
    if report.failed() {
        // As for verify: the report is the answer, the exit code is for the
        // operator's automation.
        std::process::exit(1);
    }
    Ok(())
}

/// The volume to import into: new, empty, or this registry's already.
fn open_for_import(source: &Path, into: &Path) -> Result<OciStore> {
    let same = match (source.canonicalize(), into.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    };
    if same {
        return Err(LiveError::Config(format!(
            "import into a directory of its own: {} is the source",
            into.display()
        )));
    }
    let ours = hologram_live::oci_store::Layout::resolve(into)
        .marker()
        .is_file();
    let empty = std::fs::read_dir(into).map_or(true, |mut entries| entries.next().is_none());
    if !ours && !empty {
        return Err(LiveError::Config(format!(
            "{} is neither empty nor a Hologram Registry volume",
            into.display()
        )));
    }
    std::fs::create_dir_all(into)
        .map_err(|error| LiveError::Config(format!("create {}: {error}", into.display())))?;
    let options = OpenOptions {
        create: true,
        upload_max_age: std::time::Duration::from_hours(168),
    };
    OciStore::open(into, options).map_err(|error| match error {
        OciStoreError::Locked => LiveError::Capability(format!(
            "the registry is running on {}: stop it first",
            into.display()
        )),
        other => LiveError::Config(format!("registry volume {}: {other}", into.display())),
    })
}

fn print_import(out: &mut impl std::io::Write, report: &ImportReport) -> std::io::Result<()> {
    writeln!(
        out,
        "{} repositories: {} blobs copied ({} bytes), {} linked from another repository, {} manifests, {} tags",
        report.repositories,
        report.blobs_copied,
        report.bytes_copied,
        report.blobs_linked,
        report.manifests,
        report.tags
    )?;
    let (failed, notes): (Vec<_>, Vec<_>) = report
        .problems
        .iter()
        .partition(|problem| problem.kind.fails());
    if !failed.is_empty() {
        writeln!(out, "{} not imported; the rest is:", failed.len())?;
        for problem in failed {
            writeln!(
                out,
                "  {} {}: {} ({})",
                problem.kind.as_str(),
                problem.repository,
                problem.detail,
                problem.path.display()
            )?;
        }
    }
    if !notes.is_empty() {
        writeln!(
            out,
            "{} links the source holds no bytes for (not served there either):",
            notes.len()
        )?;
        for problem in notes {
            writeln!(out, "  {}: {}", problem.repository, problem.detail)?;
        }
    }
    Ok(())
}

async fn verify(cli: &Cli, registry_config: Option<PathBuf>) -> Result<()> {
    let root = volume(&registry_file(registry_config)?)?;
    let json = cli.json;
    let report = tokio::task::spawn_blocking(move || -> Result<VerifyReport> {
        // Exit 1 is kept for damage: a locked volume is a missing capability
        // (5), a volume that cannot be read a configuration fault (2).
        let store = open_stopped(&root, "verify")?;
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
