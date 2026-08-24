use super::{helpers, Cli};
use hologram_live::error::Result;
use hologram_live::process;

pub async fn run(cli: Cli) -> Result<()> {
    let (config, path) = helpers::load(&cli)?;
    let pid = process::start_daemon(&config, &path).await?;
    if pid == 0 {
        println!("hologram daemon already running");
    } else {
        println!("hologram daemon started (pid {pid})");
    }
    Ok(())
}
