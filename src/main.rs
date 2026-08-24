#![forbid(unsafe_code)]

mod cli;

use clap::Parser;
use hologram_live::error::LiveError;

#[tokio::main]
async fn main() {
    let cli = cli::Cli::parse();
    let (tracing_config, telemetry_config) = cli.observability_config();
    let tracing_handle =
        match hologram_live::observability::init(&tracing_config, &telemetry_config) {
            Ok(handle) => handle,
            Err(error) => exit(&error),
        };
    if let Err(error) = cli.run(tracing_handle).await {
        tracing::error!(error.code = error.code(), error.message = %error, "command failed");
        exit(&error);
    }
}

fn exit(error: &LiveError) -> ! {
    eprintln!("hologram: {}: {error}", error.code());
    let status = match error {
        LiveError::Config(_) | LiveError::Protocol(_) => 2,
        LiveError::Transport(_) => 3,
        LiveError::Authentication(_) | LiveError::Authorization(_) => 4,
        LiveError::Capability(_) => 5,
        _ => 1,
    };
    std::process::exit(status);
}
