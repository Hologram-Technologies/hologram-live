#![forbid(unsafe_code)]

mod cli;

use clap::Parser;
use hologram_live::error::{ApiError, LiveError};
use std::io::Write;

#[tokio::main]
async fn main() {
    let cli = cli::Cli::parse();
    let json = cli.json;
    let (tracing_config, telemetry_config) = cli.observability_config();
    let tracing_handle =
        match hologram_live::observability::init(&tracing_config, &telemetry_config) {
            Ok(handle) => handle,
            Err(error) => exit(&error, json),
        };
    if let Err(error) = cli.run(tracing_handle).await {
        tracing::error!(error.code = error.code(), error.message = %error, "command failed");
        exit(&error, json);
    }
}

fn exit(error: &LiveError, json: bool) -> ! {
    if json {
        let encoded = serde_json::to_vec_pretty(&ApiError::from(error)).unwrap_or_else(|_| {
            br#"{"code":"LIVE_PROTOCOL_ERROR","message":"encode CLI error"}"#.to_vec()
        });
        let stdout = std::io::stdout();
        let mut stdout = stdout.lock();
        let _ = stdout.write_all(&encoded);
        let _ = stdout.write_all(b"\n");
        let _ = stdout.flush();
    } else {
        eprintln!("hologram: {}: {error}", error.code());
    }
    let status = match error {
        LiveError::Config(_) | LiveError::Protocol(_) => 2,
        LiveError::Transport(_) => 3,
        LiveError::Authentication(_) | LiveError::Authorization(_) => 4,
        LiveError::Capability(_) => 5,
        _ => 1,
    };
    std::process::exit(status);
}
