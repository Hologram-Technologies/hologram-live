use clap::{ArgAction, Parser, Subcommand};
use hologram_live::config::{AppConfig, TelemetryConfig, TracingConfig};
use hologram_live::error::Result;
use hologram_live::observability::TracingHandle;
use std::path::PathBuf;

mod config;
mod doctor;
mod files;
mod helpers;
mod history;
mod holo;
mod init;
mod modules;
mod nodes;
mod openapi;
mod registry;
mod restart;
mod route;
mod run;
mod serve;
mod start;
mod status;
mod stop;
mod tracing;
mod update;

#[derive(Debug, Clone, Parser)]
#[command(
    name = "hologram",
    version,
    about = "Extensible local and remote host for Hologram"
)]
pub struct Cli {
    /// Explicit configuration file. Defaults to ~/.config/hologram/live.toml.
    #[arg(long, global = true, env = "HOLOGRAM_CONFIG")]
    pub(crate) config: Option<PathBuf>,

    /// Emit machine-readable JSON where supported.
    #[arg(long, global = true)]
    pub(crate) json: bool,

    /// Increase tracing verbosity for this process.
    #[arg(short = 'v', long, global = true, action = ArgAction::Count)]
    verbose: u8,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Clone, Subcommand)]
enum Command {
    /// Create ~/.config/hologram/live.toml.
    Init(init::InitArgs),
    /// Run the module host in the foreground.
    Serve(serve::ServeArgs),
    /// Start the local module host in the background.
    Start,
    /// Stop the local module host.
    Stop,
    /// Restart the local module host.
    Restart,
    /// Show health for the configured local/remote route.
    Status,
    /// Inspect modules and their dependency graph.
    Modules(modules::ModulesArgs),
    /// Inspect and validate configuration.
    Config(config::ConfigArgs),
    /// Explain where an operation will run.
    Route(route::RouteArgs),
    /// Access the first content-addressed registry module.
    Registry(registry::RegistryArgs),
    /// List artifacts through the files module.
    Files,
    /// Import, verify, load, and run .holo archives.
    Holo(holo::HoloArgs),
    /// Run a .holo reference.
    Run(run::RunArgs),
    /// Manage durable conversation history.
    History(history::HistoryArgs),
    /// Minimal control-plane node inventory.
    Nodes(nodes::NodesArgs),
    /// Inspect or change the daemon tracing filter.
    Tracing(tracing::TracingArgs),
    /// Write the generated Utoipa `OpenAPI` document.
    Openapi(openapi::OpenapiArgs),
    /// Check, apply, or roll back a verified release update.
    Update(update::UpdateArgs),
    /// Validate configuration, module resolution, and local health.
    Doctor,
}

impl Cli {
    pub fn observability_config(&self) -> (TracingConfig, TelemetryConfig) {
        let bootstrap = AppConfig::load(self.config.as_deref())
            .map(|(config, _)| config)
            .unwrap_or_default();
        let mut tracing = bootstrap.tracing;
        if self.verbose == 1 {
            tracing.filter = format!("{},hologram_live=debug", tracing.filter);
        } else if self.verbose > 1 {
            tracing.filter = format!("{},hologram_live=trace", tracing.filter);
        }
        (tracing, bootstrap.telemetry)
    }

    pub async fn run(self, tracing_handle: TracingHandle) -> Result<()> {
        match self.command.clone() {
            Command::Init(args) => init::run(self, args).await,
            Command::Serve(args) => serve::run(self, args, tracing_handle).await,
            Command::Start => start::run(self).await,
            Command::Stop => stop::run(self).await,
            Command::Restart => restart::run(self).await,
            Command::Status => status::run(self).await,
            Command::Modules(args) => modules::run(self, args).await,
            Command::Config(args) => config::run(self, args).await,
            Command::Route(args) => route::run(self, args).await,
            Command::Registry(args) => registry::run(self, args).await,
            Command::Files => files::run(self).await,
            Command::Holo(args) => holo::run(self, args).await,
            Command::Run(args) => run::run(self, args).await,
            Command::History(args) => history::run(self, args).await,
            Command::Nodes(args) => nodes::run(self, args).await,
            Command::Tracing(args) => tracing::run(self, args).await,
            Command::Openapi(args) => openapi::run(self, args).await,
            Command::Update(args) => update::run(self, args).await,
            Command::Doctor => doctor::run(self).await,
        }
    }
}
