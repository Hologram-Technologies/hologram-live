use super::{helpers, Cli};
use hologram_live::error::Result;
use hologram_live::process;

pub async fn run(cli: Cli) -> Result<()> {
    let (config, path) = helpers::load(&cli)?;
    let pid = process::start_daemon(&config, &path).await?;
    if pid == 0 {
        helpers::message(&cli, "already_running", "hologram daemon already running")
    } else if cli.json {
        helpers::print(
            &cli,
            &serde_json::json!({ "status": "started", "pid": pid }),
        )
    } else {
        helpers::message(
            &cli,
            "started",
            format!("hologram daemon started (pid {pid})"),
        )
    }
}
