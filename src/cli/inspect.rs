use super::{holo, Cli};
use clap::Args;
use hologram_live::error::Result;

#[derive(Debug, Clone, Args)]
pub struct InspectArgs {
    /// Catalog kappa, or a local .holo file.
    pub reference: String,
    /// Verify signatures and digest tree during inspection.
    #[arg(long)]
    pub verify: bool,
}

pub async fn run(cli: Cli, args: InspectArgs) -> Result<()> {
    holo::inspect(&cli, args.reference, args.verify).await
}
