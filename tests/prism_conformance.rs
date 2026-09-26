#![forbid(unsafe_code)]

use hologram_live::{
    dispatchBytes, dispatchString, executeCommand, parseCliCommand, CliCommand,
};

#[test]
fn test_prism_cli_commands_coverage() {
    let commands = [
        ("run", CliCommand::Run, r#"{"command":"run","status":"ok"}"#),
        ("serve", CliCommand::Serve, r#"{"command":"serve","status":"ok"}"#),
        ("pull", CliCommand::Pull, r#"{"command":"pull","status":"ok"}"#),
        ("push", CliCommand::Push, r#"{"command":"push","status":"ok"}"#),
        ("inspect", CliCommand::Inspect, r#"{"holo_version":4,"magic":"HOLO"}"#),
        ("chat", CliCommand::Chat, r#"{"command":"chat","status":"ok"}"#),
        ("nodes", CliCommand::Nodes, r#"{"command":"nodes","status":"ok"}"#),
        ("status", CliCommand::Status, r#"{"status":"ready","version":"1.0.0","engine":"prismpm"}"#),
        ("stop", CliCommand::Stop, r#"{"command":"stop","status":"ok"}"#),
        ("restart", CliCommand::Restart, r#"{"command":"restart","status":"ok"}"#),
        ("update", CliCommand::Update, r#"{"command":"update","status":"ok"}"#),
        ("init", CliCommand::Init, r#"{"command":"init","status":"ok"}"#),
        ("compile", CliCommand::Compile, r#"{"command":"compile","status":"ok"}"#),
        ("files", CliCommand::Files, r#"{"command":"files","status":"ok"}"#),
        ("doctor", CliCommand::Doctor, r#"{"command":"doctor","status":"ok"}"#),
        ("ai", CliCommand::Ai, r#"{"service":"hologram-ai","cost_model":"uor-prism","status":"optimal"}"#),
        ("plugins", CliCommand::Plugins, r#"{"command":"plugins","status":"ok"}"#),
        ("oci", CliCommand::Oci, r#"{"command":"oci","status":"ok"}"#),
        ("models", CliCommand::Models, r#"{"command":"models","status":"ok"}"#),
        ("route", CliCommand::Route, r#"{"command":"route","status":"ok"}"#),
        ("app", CliCommand::App, r#"{"command":"app","status":"ok"}"#),
        ("config", CliCommand::Config, r#"{"command":"config","status":"ok"}"#),
        ("tracing", CliCommand::Tracing, r#"{"command":"tracing","status":"ok"}"#),
        ("history", CliCommand::History, r#"{"command":"history","status":"ok"}"#),
        ("openapi", CliCommand::Openapi, r#"{"command":"openapi","status":"ok"}"#),
        ("verify", CliCommand::Verify, r#"{"command":"verify","status":"ok"}"#),
        ("plan", CliCommand::Plan, r#"{"command":"plan","status":"ok"}"#),
        ("help", CliCommand::Help, r#"{"help":"hologram <command>","commands":28}"#),
    ];

    assert_eq!(commands.len(), 28, "Must test exactly all 28 modeled CLI commands");

    for (name, expected_cmd, expected_response) in commands {
        let parsed = parseCliCommand(name.to_string());
        assert_eq!(parsed, expected_cmd, "Command '{name}' failed to parse");
        let result = executeCommand(parsed);
        assert_eq!(result, expected_response, "Command '{name}' produced unexpected execution response");

        let dispatched_str = dispatchString(name.to_string());
        assert_eq!(dispatched_str, expected_response, "dispatchString('{name}') failed");

        let dispatched_bytes = dispatchBytes(name.as_bytes().to_vec());
        assert_eq!(
            String::from_utf8(dispatched_bytes).unwrap(),
            expected_response,
            "dispatchBytes('{name}') failed"
        );
    }
}

#[test]
fn test_prism_unknown_and_malformed() {
    let unknown_resp = dispatchString("invalid_command_xyz".to_string());
    assert_eq!(unknown_resp, r#"{"error":"unknown command"}"#);

    let malformed_bytes = vec![0xFF, 0xFE, 0xFD];
    let malformed_resp = dispatchBytes(malformed_bytes);
    assert_eq!(
        String::from_utf8(malformed_resp).unwrap(),
        r#"{"error":"malformed-utf8"}"#
    );
}

#[test]
fn test_hologram_ai_uor_cost_model_optimal() {
    let ai_resp = dispatchString("ai".to_string());
    let v: serde_json::Value = serde_json::from_str(&ai_resp).expect("Valid JSON response");
    assert_eq!(v["service"], "hologram-ai");
    assert_eq!(v["cost_model"], "uor-prism");
    assert_eq!(v["status"], "optimal");
}

#[test]
fn test_hologram_v4_container_oracle() {
    let holo_path = std::path::Path::new(".prism/build/236ff75a002b49ca0a1ee6090c91ebacedb4d07affb9649d33809ea67ceb88fe/Hologram Live.holo");
    let bytes = std::fs::read(holo_path).expect("Read Hologram Live.holo");
    assert!(bytes.len() >= 16);
    assert_eq!(&bytes[0..4], b"HOLO");
    assert_eq!(&bytes[4..6], b"\x04\x00");
    hologram_live::holo_format::require_current(&bytes).expect("Holo v4 validation succeeds");
}

