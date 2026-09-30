#![forbid(unsafe_code)]

mod cli;

use clap::Parser;
use hologram_live::error::{ApiError, LiveError};
use std::io::Write;

#[tokio::main]
async fn main() {
    let mut raw_args: Vec<String> = std::env::args().collect();
    let is_prism = raw_args.iter().any(|arg| arg == "--prism")
        || std::env::var("HOLOGRAM_ENGINE").as_deref() == Ok("prismpm");

    let is_json = raw_args.iter().any(|arg| arg == "--json");

    if is_prism {
        std::env::set_var("HOLOGRAM_ENGINE", "prismpm");
        raw_args.retain(|arg| arg != "--prism");

        // Validate command route against PrismPM declarative model
        let subcmd = raw_args
            .iter()
            .skip(1)
            .find(|arg| !arg.starts_with('-'))
            .map_or("help", std::string::String::as_str);

        // Canonicalize CLI subcommands to their formal PrismPM capability routes:
        // - `start` is the daemonized lifecycle variant of `serve`
        // - `modules` is the plugin/component inventory capability (`plugins`)
        // - `registry` is the OCI distribution storage capability (`oci`)
        // - `holo` is the container inspection and execution capability (`inspect`)
        // - `server` is the API gateway server capability (`serve`)
        // - `cas` is the content-addressed storage capability (`files`)
        let canonical_cmd = match subcmd {
            "start" | "server" => "serve",
            "modules" => "plugins",
            "registry" => "oci",
            "holo" => "inspect",
            "cas" => "files",
            other => other,
        };

        let prism_cmd = hologram_live::parseCliCommand(canonical_cmd.to_string());
        if prism_cmd == hologram_live::CliCommand::Unknown {
            let error = LiveError::Capability(format!(
                "command '{subcmd}' is not modeled or permitted by the PrismPM system architecture"
            ));
            exit(&error, is_json);
        }
    }

    let raw_os_args: Vec<std::ffi::OsString> =
        raw_args.into_iter().map(std::ffi::OsString::from).collect();

    #[cfg(feature = "oci")]
    let cli = {
        // The reference image's command names, when this binary is linked as
        // `registry` or `entrypoint.sh`.
        match cli::registry_argv::rewrite(&raw_os_args) {
            None => cli::Cli::parse_from(&raw_os_args),
            Some(cli::registry_argv::Rewritten::Args(rewritten)) => cli::Cli::parse_from(rewritten),
            Some(cli::registry_argv::Rewritten::Print(text)) => {
                println!("{text}");
                std::process::exit(0);
            }
            Some(cli::registry_argv::Rewritten::Refuse(text)) => {
                eprintln!("{text}");
                std::process::exit(2);
            }
        }
    };
    #[cfg(not(feature = "oci"))]
    let cli = cli::Cli::parse_from(&raw_os_args);
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
