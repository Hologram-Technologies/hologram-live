#![forbid(unsafe_code)]

use hologram_live::{dispatchBytes, dispatchString, executeCommand, parseCliCommand, CliCommand};

#[test]
fn test_prism_cli_commands_coverage() {
    let commands = [
        ("run", CliCommand::Run, r#"{"command":"run","status":"ok"}"#),
        (
            "serve",
            CliCommand::Serve,
            r#"{"command":"serve","status":"ok"}"#,
        ),
        (
            "pull",
            CliCommand::Pull,
            r#"{"command":"pull","status":"ok"}"#,
        ),
        (
            "push",
            CliCommand::Push,
            r#"{"command":"push","status":"ok"}"#,
        ),
        (
            "inspect",
            CliCommand::Inspect,
            r#"{"holo_version":4,"magic":"HOLO"}"#,
        ),
        (
            "chat",
            CliCommand::Chat,
            r#"{"command":"chat","status":"ok"}"#,
        ),
        (
            "nodes",
            CliCommand::Nodes,
            r#"{"command":"nodes","status":"ok"}"#,
        ),
        (
            "status",
            CliCommand::Status,
            r#"{"status":"ready","version":"1.0.0","engine":"prismpm"}"#,
        ),
        (
            "stop",
            CliCommand::Stop,
            r#"{"command":"stop","status":"ok"}"#,
        ),
        (
            "restart",
            CliCommand::Restart,
            r#"{"command":"restart","status":"ok"}"#,
        ),
        (
            "update",
            CliCommand::Update,
            r#"{"command":"update","status":"ok"}"#,
        ),
        (
            "init",
            CliCommand::Init,
            r#"{"command":"init","status":"ok"}"#,
        ),
        (
            "compile",
            CliCommand::Compile,
            r#"{"command":"compile","status":"ok"}"#,
        ),
        (
            "files",
            CliCommand::Files,
            r#"{"command":"files","status":"ok"}"#,
        ),
        (
            "doctor",
            CliCommand::Doctor,
            r#"{"command":"doctor","status":"ok"}"#,
        ),
        (
            "ai",
            CliCommand::Ai,
            r#"{"service":"hologram-ai","cost_model":"uor-prism","status":"optimal"}"#,
        ),
        (
            "plugins",
            CliCommand::Plugins,
            r#"{"command":"plugins","status":"ok"}"#,
        ),
        ("oci", CliCommand::Oci, r#"{"command":"oci","status":"ok"}"#),
        (
            "models",
            CliCommand::Models,
            r#"{"command":"models","status":"ok"}"#,
        ),
        (
            "route",
            CliCommand::Route,
            r#"{"command":"route","status":"ok"}"#,
        ),
        ("app", CliCommand::App, r#"{"command":"app","status":"ok"}"#),
        (
            "config",
            CliCommand::Config,
            r#"{"command":"config","status":"ok"}"#,
        ),
        (
            "tracing",
            CliCommand::Tracing,
            r#"{"command":"tracing","status":"ok"}"#,
        ),
        (
            "history",
            CliCommand::History,
            r#"{"command":"history","status":"ok"}"#,
        ),
        (
            "openapi",
            CliCommand::Openapi,
            r#"{"command":"openapi","status":"ok"}"#,
        ),
        (
            "verify",
            CliCommand::Verify,
            r#"{"command":"verify","status":"ok"}"#,
        ),
        (
            "plan",
            CliCommand::Plan,
            r#"{"command":"plan","status":"ok"}"#,
        ),
        (
            "help",
            CliCommand::Help,
            r#"{"help":"hologram <command>","commands":28}"#,
        ),
    ];

    assert_eq!(
        commands.len(),
        28,
        "Must test exactly all 28 modeled CLI commands"
    );

    for (name, expected_cmd, expected_response) in commands {
        let parsed = parseCliCommand(name.to_string());
        assert_eq!(parsed, expected_cmd, "Command '{name}' failed to parse");
        let result = executeCommand(parsed);
        assert_eq!(
            result, expected_response,
            "Command '{name}' produced unexpected execution response"
        );

        let dispatched_str = dispatchString(name.to_string());
        assert_eq!(
            dispatched_str, expected_response,
            "dispatchString('{name}') failed"
        );

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

/// `None` when no plan has been projected yet — a fresh checkout has no
/// `.prism/build`, and an oracle for a projection that does not exist has
/// nothing to check, so the dependent tests skip rather than fail.
fn find_latest_build_dir() -> Option<std::path::PathBuf> {
    let build_dir = std::path::Path::new(".prism/build");
    let mut entries: Vec<_> = std::fs::read_dir(build_dir)
        .ok()?
        .filter_map(std::result::Result::ok)
        .filter(|e| e.path().is_dir())
        .collect();
    entries.sort_by_key(|e| e.metadata().and_then(|m| m.modified()).ok());
    entries.last().map(|e| e.path())
}

#[test]
fn test_hologram_v4_container_oracle() {
    let Some(build_dir) = find_latest_build_dir() else {
        return;
    };
    let holo_path = build_dir.join("Hologram Live.holo");
    let bytes = std::fs::read(holo_path).expect("Read Hologram Live.holo");
    assert!(bytes.len() >= 16);
    assert_eq!(&bytes[0..4], b"HOLO");
    assert_eq!(&bytes[4..6], b"\x04\x00");
    hologram_live::holo_format::require_current(&bytes).expect("Holo v4 validation succeeds");
}

#[test]
fn test_prism_system_projections_coverage() {
    let Some(build_dir) = find_latest_build_dir() else {
        return;
    };
    let proj_dir = build_dir.join("projections");
    assert!(
        proj_dir.exists(),
        "projections directory must exist in build output"
    );

    let required_projections = [
        "asyncapi.json",
        "capability-coverage.json",
        "cloudevents.schema.json",
        "compose.json",
        "history.sql",
        "kubernetes.json",
        "openapi.json",
        "opentelemetry-collector.json",
        "runtime-contract.json",
        "spdx.json",
        "system-validation-certificate.json",
    ];

    for name in required_projections {
        let path = proj_dir.join(name);
        assert!(
            path.exists(),
            "Projection {name} must exist in build output"
        );
        let content = std::fs::read_to_string(&path).expect("Read projection content");
        assert!(
            !content.trim().is_empty(),
            "Projection {name} must not be empty"
        );

        if path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
        {
            let _: serde_json::Value = serde_json::from_str(&content)
                .unwrap_or_else(|e| panic!("Projection {name} must be valid JSON: {e}"));
        }
    }
}

#[test]
fn test_prism_system_validation_certificate() {
    let Some(build_dir) = find_latest_build_dir() else {
        return;
    };
    let cert_path = build_dir.join("projections/system-validation-certificate.json");
    let cert_str = std::fs::read_to_string(&cert_path).expect("Read certificate");
    let cert: serde_json::Value = serde_json::from_str(&cert_str).expect("Parse certificate");

    assert_eq!(
        cert["schema"], "prismpm/system-validation-certificate/1",
        "Certificate must match system validation schema"
    );

    let required_relations = [
        "capability_satisfaction",
        "closure",
        "compatibility",
        "deployment_order",
        "evidence_closure",
        "license_closure",
        "migration_order",
        "referential_integrity",
        "release_completeness",
        "rollback_safety",
        "secret_flow",
        "uniqueness",
    ];

    for relation in required_relations {
        let rel_obj = &cert[relation];
        assert!(
            rel_obj.is_object(),
            "Relation {relation} must be an object in validation certificate"
        );
        let bound = rel_obj["bound"]
            .as_u64()
            .unwrap_or_else(|| panic!("Relation {relation} bound must be a non-negative integer"));
        assert!(
            bound > 0,
            "Relation {relation} bound must be strictly positive"
        );
        let values = rel_obj["values"]
            .as_array()
            .unwrap_or_else(|| panic!("Relation {relation} values must be an array"));
        assert!(
            !values.is_empty(),
            "Relation {relation} values array must not be empty"
        );
        for val in values {
            let v = val.as_u64().expect("Relation value must be an integer");
            assert!(
                v < bound,
                "Relation value {v} must be strictly below bound {bound}"
            );
        }
    }
}

#[test]
fn test_prism_stakeholder_viewpoints_and_architecture() {
    let Some(build_dir) = find_latest_build_dir() else {
        return;
    };
    let system_path = build_dir.join("system.prism.json");
    let system_str = std::fs::read_to_string(&system_path).expect("Read system.prism.json");
    let system: serde_json::Value =
        serde_json::from_str(&system_str).expect("Parse system.prism.json");

    assert_eq!(system["schema"], "prismpm/system-model/1");

    // Check components: 5 core components modeled without arbitrary plumbing
    let components = system["components"].as_array().expect("components array");
    let comp_ids: Vec<&str> = components.iter().filter_map(|c| c["id"].as_str()).collect();
    assert!(comp_ids.contains(&"server"));
    assert!(comp_ids.contains(&"inference-engine"));
    assert!(comp_ids.contains(&"cas-store"));
    assert!(comp_ids.contains(&"migrate"));
    assert!(comp_ids.contains(&"telemetry"));

    // Check architecture viewpoints for modeled stakeholders
    let architecture = system["architecture"]
        .as_array()
        .expect("architecture array");
    let arch_ids: Vec<&str> = architecture
        .iter()
        .filter_map(|a| a["id"].as_str())
        .collect();
    assert!(
        arch_ids.contains(&"arch-stakeholder-operator"),
        "Must model Edge AI Operator"
    );
    assert!(
        arch_ids.contains(&"arch-stakeholder-developer"),
        "Must model Model Developer"
    );
    assert!(
        arch_ids.contains(&"arch-stakeholder-security"),
        "Must model Security Auditor"
    );
    assert!(
        arch_ids.contains(&"arch-decision-holo-v4"),
        "Must model Holo v4 decision"
    );
    assert!(
        arch_ids.contains(&"arch-decision-uor-cost-model"),
        "Must model UOR cost-model decision"
    );

    // Check target bindings
    let targets = system["targets"].as_array().expect("targets array");
    let target_ids: Vec<&str> = targets.iter().filter_map(|t| t["id"].as_str()).collect();
    assert!(target_ids.contains(&"target-compose"));
    assert!(target_ids.contains(&"target-kubernetes"));
}

#[test]
#[allow(clippy::cast_precision_loss)]
fn test_prism_router_throughput_and_latency() {
    let iterations = 100_000;
    let commands = ["run", "serve", "ai", "status", "doctor", "unknown"];
    let start = std::time::Instant::now();
    for _ in 0..iterations {
        for cmd in &commands {
            let parsed = parseCliCommand((*cmd).to_string());
            std::hint::black_box(parsed);
        }
    }
    let elapsed = start.elapsed();
    let total_ops = iterations * commands.len();
    let nanos_per_op = elapsed.as_nanos() as f64 / total_ops as f64;
    let ops_per_sec = total_ops as f64 / elapsed.as_secs_f64();
    println!(
        "PrismPM router: {total_ops} dispatches in {elapsed:?} ({nanos_per_op:.1} ns/op, {ops_per_sec:.0} ops/sec)"
    );
    assert!(
        nanos_per_op < 1_000.0,
        "Prism router dispatch must be sub-microsecond"
    );
}
