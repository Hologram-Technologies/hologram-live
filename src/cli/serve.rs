use super::{helpers, Cli};
use clap::Args;
use hologram_live::app::AppState;
use hologram_live::error::Result;
use hologram_live::observability::TracingHandle;
use hologram_live::{process, server};

#[derive(Debug, Clone, Args)]
pub struct ServeArgs {
    #[arg(long)]
    listen: Option<String>,
}

pub async fn run(cli: Cli, args: ServeArgs, tracing: TracingHandle) -> Result<()> {
    let (mut config, _) = helpers::load(&cli)?;
    if let Some(listen) = args.listen {
        config.server.listen = listen;
    }
    config.validate()?;
    let listen = config.server.listen.clone();
    let declared: Vec<String> = config
        .holo
        .resident
        .iter()
        .map(|entry| entry.kappa.clone())
        .collect();
    let _guard = process::DaemonGuard::acquire(&config)?;
    let state = AppState::build(config, tracing.clone()).await?;
    // Load operator-declared resident applications before binding the
    // listener, so the daemon does not report ready until they are
    // invocable. Load time delays readiness probes; keep declarations
    // small. Failures skip only the failing entry and are already logged.
    if !declared.is_empty() {
        let outcomes = state.holo_runtime().load_declared(&declared).await;
        let loaded = outcomes
            .iter()
            .filter(|(_, outcome)| outcome.is_ok())
            .count();
        tracing::info!(
            loaded,
            declared = outcomes.len(),
            "declared resident holo applications processed"
        );
    }
    let result = server::serve_with_ready(state, move || {
        if cli.json {
            helpers::print(
                &cli,
                &serde_json::json!({ "status": "serving", "listen": listen }),
            )
        } else {
            Ok(())
        }
    })
    .await;
    if let Err(error) = tracing.force_flush() {
        tracing::warn!(error = %error, "failed to flush telemetry during shutdown");
    }
    result
}
