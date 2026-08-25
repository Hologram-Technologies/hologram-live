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
    let _guard = process::DaemonGuard::acquire(&config)?;
    let state = AppState::build(config, tracing.clone()).await?;
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
