use clap::{ArgAction, Parser, Subcommand};
use hologram_live::config::{AppConfig, TelemetryConfig, TracingConfig};
use hologram_live::error::Result;
use hologram_live::observability::TracingHandle;
use std::path::PathBuf;

mod ai;
mod app;
mod chat;
mod compile;
mod config;
mod doctor;
mod files;
mod helpers;
mod history;
mod holo;
mod init;
mod models;
mod modules;
mod nodes;
#[cfg(feature = "oci")]
mod oci;
mod openapi;
mod plugins;
mod pull;
mod push;
mod registry;
#[cfg(feature = "oci")]
pub(crate) mod registry_argv;
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

    /// Emit machine-readable JSON for every command result.
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
    /// Inspect AI model services packaged in .holo archives.
    Ai(ai::AiArgs),
    /// Create and manage Hologram application source manifests.
    App(Box<app::AppArgs>),
    /// Create ~/.config/hologram/live.toml.
    Init(init::InitArgs),
    /// Compile a JSON application manifest into a self-contained .holo archive.
    Compile(compile::CompileArgs),
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
    /// Operator commands of the Docker-compatible registry (`garbage-collect`, `verify`, `import`).
    #[cfg(feature = "oci")]
    Oci(oci::OciArgs),
    /// Store, list, and download file objects.
    Files(files::FilesArgs),
    /// Import, verify, load, and run .holo archives.
    Holo(holo::HoloArgs),
    /// Fetch a named .holo artifact from the configured registry.
    Pull(pull::PullArgs),
    /// Publish a local .holo archive to the configured registry under a name.
    Push(push::PushArgs),
    /// Run a .holo reference.
    Run(run::RunArgs),
    /// Manage durable conversation history.
    History(history::HistoryArgs),
    /// Chat through an enabled conversation-backed module.
    Chat(chat::ChatArgs),
    /// List, import, and remove inference models.
    Models(models::ModelsArgs),
    /// Minimal control-plane node inventory.
    Nodes(nodes::NodesArgs),
    /// List and invoke subprocess plugin modules.
    Plugins(plugins::PluginsArgs),
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
    /// The registry configuration `serve` was given, if any.
    #[cfg_attr(
        not(feature = "oci"),
        allow(clippy::unused_self, reason = "only the registry build has one")
    )]
    fn registry_config(&self) -> Option<PathBuf> {
        #[cfg(feature = "oci")]
        if let Command::Serve(args) = &self.command {
            return args.registry_config.clone();
        }
        None
    }

    pub fn observability_config(&self) -> (TracingConfig, TelemetryConfig) {
        let registry_config = self.registry_config();
        let bootstrap = if registry_config.is_some() && self.config.is_none() {
            // Registry mode: logging never comes from a user's
            // ~/.config/hologram/live.toml, unless --config names a file.
            AppConfig::default()
        } else {
            AppConfig::load_for_bootstrap(self.config.as_deref())
                .map(|(config, _)| config)
                .unwrap_or_default()
        };
        let mut tracing = bootstrap.tracing;
        #[cfg(feature = "oci")]
        if let Some(file) = &registry_config {
            // A bad file is reported by `serve`, which loads the same settings.
            if let Ok(settings) = hologram_live::registry_compat::load(Some(file), std::env::vars())
            {
                tracing = hologram_live::registry_compat::tracing_config(&settings, tracing);
            }
        }
        if self.verbose == 1 {
            tracing.filter = format!("{},hologram_live=debug", tracing.filter);
        } else if self.verbose > 1 {
            tracing.filter = format!("{},hologram_live=trace", tracing.filter);
        }
        (tracing, bootstrap.telemetry)
    }

    pub async fn run(self, tracing_handle: TracingHandle) -> Result<()> {
        match self.command.clone() {
            Command::Ai(args) => ai::run(self, args).await,
            Command::App(args) => app::run(self, *args).await,
            Command::Init(args) => init::run(self, args).await,
            Command::Compile(args) => compile::run(self, args).await,
            Command::Serve(args) => serve::run(self, args, tracing_handle).await,
            Command::Start => start::run(self).await,
            Command::Stop => stop::run(self).await,
            Command::Restart => restart::run(self).await,
            Command::Status => status::run(self).await,
            Command::Modules(args) => modules::run(self, args).await,
            Command::Config(args) => config::run(self, args).await,
            Command::Route(args) => route::run(self, args).await,
            Command::Registry(args) => registry::run(self, args).await,
            #[cfg(feature = "oci")]
            Command::Oci(args) => oci::run(self, args).await,
            Command::Files(args) => files::run(self, args).await,
            Command::Holo(args) => holo::run(self, args).await,
            Command::Pull(args) => pull::run(self, args).await,
            Command::Push(args) => push::run(self, args).await,
            Command::Run(args) => run::run(self, args).await,
            Command::History(args) => history::run(self, args).await,
            Command::Chat(args) => chat::run(self, args).await,
            Command::Models(args) => models::run(self, args).await,
            Command::Nodes(args) => nodes::run(self, args).await,
            Command::Plugins(args) => plugins::run(self, args).await,
            Command::Tracing(args) => tracing::run(self, args).await,
            Command::Openapi(args) => openapi::run(self, args).await,
            Command::Update(args) => update::run(self, args).await,
            Command::Doctor => doctor::run(self).await,
        }
    }
}
