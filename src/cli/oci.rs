//! `hologram oci …`: the registry's operator commands.
//!
//! Declared now so the reference's command names reach them (`registry
//! garbage-collect` becomes `hologram oci garbage-collect`); each is built in
//! the operator-commands work (plan, P8) and until then says so.

use super::Cli;
use clap::{Args, Subcommand};
use hologram_live::error::{LiveError, Result};
use std::path::PathBuf;

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
    /// Re-hash every blob and report any that do not match their digest.
    Verify {
        #[arg(long)]
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

pub async fn run(_cli: Cli, args: OciArgs) -> Result<()> {
    let name = match args.command {
        OciCommand::GarbageCollect { .. } => "garbage-collect",
        OciCommand::Verify { .. } => "verify",
        OciCommand::Import { .. } => "import",
    };
    Err(LiveError::Capability(format!(
        "hologram oci {name} is not built yet (plan P8)"
    )))
}
