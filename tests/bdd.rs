use cucumber::{given, then, when, World as _};
use hologram_live::holo::inspect_bytes;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

#[derive(Debug, Default, cucumber::World)]
struct BddWorld {
    manifest: Option<PathBuf>,
    output_path: Option<PathBuf>,
    command_output: Option<Output>,
    temporary: Option<tempfile::TempDir>,
    home: Option<tempfile::TempDir>,
    conversation_id: Option<String>,
    sent_message: Option<String>,
    kappa: Option<String>,
    run_result: Option<serde_json::Value>,
    plan_result: Option<serde_json::Value>,
    plugin_list: Option<serde_json::Value>,
    model_id: Option<String>,
    development_grant: Option<PathBuf>,
}

impl Drop for BddWorld {
    fn drop(&mut self) {
        if let Some(home) = &self.home {
            let _ = Command::new(env!("CARGO_BIN_EXE_hologram"))
                .arg("stop")
                .env("HOME", home.path())
                .output();
        }
    }
}

#[given("the example view application manifest")]
fn example_manifest(world: &mut BddWorld) {
    world.manifest = Some(
        workspace_root()
            .join("features")
            .join("fixtures")
            .join("view-app")
            .join("hologram.json"),
    );
    world.temporary = Some(tempfile::tempdir().expect("create scenario directory"));
}

#[given("the example wasm application manifest")]
fn wasm_manifest(world: &mut BddWorld) {
    world.manifest = Some(
        workspace_root()
            .join("features")
            .join("fixtures")
            .join("wasm-app")
            .join("hologram.json"),
    );
    world.temporary = Some(tempfile::tempdir().expect("create scenario directory"));
}

#[given("a Wasm application with a custom manifest entrypoint")]
fn wasm_manifest_with_custom_entry(world: &mut BddWorld) {
    let temporary = tempfile::tempdir().expect("create scenario directory");
    let fixture =
        std::fs::read_to_string(workspace_root().join("features/fixtures/wasm-app/transform.wat"))
            .expect("read Wasm fixture");
    std::fs::write(
        temporary.path().join("transform.wat"),
        fixture.replace("(export \"holo_run\")", "(export \"transform\")"),
    )
    .expect("write custom-entry fixture");
    std::fs::write(
        temporary.path().join("hologram.json"),
        r#"{
          "schema_version": 4,
          "primary": 0,
          "layers": [{"kind":"wasm","path":"transform.wat","entry":"transform","contract":"hologram:guest/core-wasm@1"}]
        }"#,
    )
    .expect("write manifest");
    world.manifest = Some(temporary.path().join("hologram.json"));
    world.temporary = Some(temporary);
}

#[given("a Component v1 application manifest")]
fn component_v1_manifest(world: &mut BddWorld) {
    let temporary = tempfile::tempdir().expect("create scenario directory");
    std::fs::copy(
        workspace_root().join("tests/fixtures/component-echo/echo.wat"),
        temporary.path().join("application.component.wasm"),
    )
    .expect("copy component fixture");
    std::fs::write(
        temporary.path().join("hologram.json"),
        r#"{
          "schema_version": 4,
          "primary": 0,
          "layers": [{
            "kind":"wasm",
            "path":"application.component.wasm",
            "entry":"run",
            "contract":"hologram:guest/component@1"
          }]
        }"#,
    )
    .expect("write component manifest");
    world.manifest = Some(temporary.path().join("hologram.json"));
    world.temporary = Some(temporary);
}

#[given("a Wasm application that requests network fetch")]
fn wasm_manifest_with_network_request(world: &mut BddWorld) {
    let temporary = tempfile::tempdir().expect("create scenario directory");
    std::fs::copy(
        workspace_root().join("features/fixtures/wasm-app/transform.wat"),
        temporary.path().join("transform.wat"),
    )
    .expect("copy Wasm fixture");
    let capabilities = r#"{"schema_version":1,"network_fetch":true}"#;
    std::fs::write(temporary.path().join("capabilities.json"), capabilities)
        .expect("write capability request");
    let grant = temporary.path().join("development-grant.json");
    std::fs::write(&grant, capabilities).expect("write development grant");
    std::fs::write(
        temporary.path().join("hologram.json"),
        r#"{
          "schema_version": 4,
          "primary": 0,
          "requires": "capabilities.json",
          "layers": [{"kind":"wasm","path":"transform.wat","entry":"holo_run","contract":"hologram:guest/core-wasm@1"}]
        }"#,
    )
    .expect("write manifest");
    world.manifest = Some(temporary.path().join("hologram.json"));
    world.development_grant = Some(grant);
    world.temporary = Some(temporary);
}

#[given("a new application directory")]
fn new_application_directory(world: &mut BddWorld) {
    let temporary = tempfile::tempdir().expect("create application directory");
    let fixture = workspace_root().join("features/fixtures/wasm-app/transform.wat");
    std::fs::copy(fixture, temporary.path().join("transform.wat")).expect("copy Wasm fixture");
    world.temporary = Some(temporary);
}

#[when("I initialize a Wasm application manifest")]
fn initialize_wasm_manifest(world: &mut BddWorld) {
    let directory = world
        .temporary
        .as_ref()
        .expect("application directory")
        .path();
    let output = Command::new(env!("CARGO_BIN_EXE_hologram"))
        .arg("--json")
        .arg("app")
        .arg("init")
        .arg(directory)
        .arg("--kind")
        .arg("wasm")
        .arg("--path")
        .arg("transform.wat")
        .arg("--entry")
        .arg("holo_run")
        .output()
        .expect("initialize app manifest");
    assert!(
        output.status.success(),
        "app init failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("parse init report");
    assert_eq!(report["layer_count"], 1);
    world.manifest = Some(directory.join("hologram.json"));
}

#[then("the generated manifest is valid")]
fn generated_manifest_is_valid(world: &mut BddWorld) {
    let bytes = std::fs::read(world.manifest.as_ref().expect("manifest")).expect("read manifest");
    let manifest: hologram_live::compile::CompileManifest =
        serde_json::from_slice(&bytes).expect("parse manifest");
    hologram_live::compile::validate_compile_manifest(&manifest).expect("validate manifest");
    assert_eq!(manifest.primary, Some(0));
}

#[when("I import the compiled archive")]
fn import_archive(world: &mut BddWorld) {
    let path = world
        .output_path
        .as_ref()
        .expect("compiled archive")
        .to_string_lossy()
        .into_owned();
    let output = run_cli(world, &["--json", "holo", "import", &path]);
    assert!(
        output.status.success(),
        "holo import failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let record: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("parse import output");
    world.kappa = Some(record["kappa"].as_str().expect("kappa").to_owned());
}

#[when("I plan the compiled archive directly")]
fn plan_local_archive(world: &mut BddWorld) {
    let output = Command::new(env!("CARGO_BIN_EXE_hologram"))
        .arg("--json")
        .arg("holo")
        .arg("plan")
        .arg(world.output_path.as_ref().expect("compiled archive"))
        .output()
        .expect("plan local archive");
    assert!(
        output.status.success(),
        "local holo plan failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    world.plan_result = Some(serde_json::from_slice(&output.stdout).expect("parse plan output"));
}

#[when("I plan the imported archive")]
fn plan_imported_archive(world: &mut BddWorld) {
    let kappa = world.kappa.clone().expect("kappa");
    let output = run_cli(world, &["--json", "holo", "plan", &kappa]);
    assert!(
        output.status.success(),
        "resident holo plan failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    world.plan_result = Some(serde_json::from_slice(&output.stdout).expect("parse plan output"));
}

#[then("the direct plan is runnable without exposing payload bytes")]
fn direct_plan_is_payload_free(world: &mut BddWorld) {
    let plan = world.plan_result.as_ref().expect("plan result");
    assert_eq!(plan["execution_target"], "direct");
    assert_eq!(plan["packaging"], "fat");
    assert_eq!(plan["runnable"], true);
    assert_eq!(plan["layers"][0]["provider"]["status"], "available");
    assert!(plan["layers"][0].get("content").is_none());
    assert!(plan["layers"][0].get("bytes").is_none());
}

#[then("the direct plan reports that the portable View surface is unavailable")]
fn direct_plan_reports_unavailable_view_surface(world: &mut BddWorld) {
    let plan = world.plan_result.as_ref().expect("plan result");
    assert_eq!(plan["execution_target"], "direct");
    assert_eq!(plan["runnable"], false);
    assert_eq!(plan["layers"][0]["provider"]["status"], "unavailable");
    let reason = plan["layers"][0]["provider"]["reason"]
        .as_str()
        .expect("provider reason");
    assert!(reason.contains("portable View surface"), "{reason}");
    assert!(reason.contains("direct/headless"), "{reason}");
    assert!(plan["blockers"]
        .as_array()
        .expect("blockers")
        .iter()
        .any(|blocker| blocker["kind"] == "provider_unavailable"
            && blocker["error_code"] == "LIVE_CAPABILITY_MISSING"));
}

#[then("the component contract selects the bounded component provider")]
fn component_plan_selects_bounded_provider(world: &mut BddWorld) {
    let plan = world.plan_result.as_ref().expect("plan result");
    assert_eq!(plan["execution_target"], "direct");
    assert_eq!(plan["runnable"], true);
    assert_eq!(plan["layers"][0]["contract"], "hologram:guest/component@1");
    assert_eq!(plan["layers"][0]["provider"]["status"], "available");
    assert_eq!(
        plan["layers"][0]["provider"]["name"],
        "wasmtime-component-direct"
    );
    assert!(plan["layers"][0]["provider"]["reason"].is_null());
    assert_eq!(plan["blockers"].as_array().expect("blockers").len(), 0);
}

#[then("the resident plan identifies the imported archive")]
fn resident_plan_identifies_archive(world: &mut BddWorld) {
    let plan = world.plan_result.as_ref().expect("plan result");
    assert_eq!(plan["execution_target"], "resident");
    assert_eq!(
        plan["archive_kappa"].as_str(),
        Some(world.kappa.as_deref().expect("kappa"))
    );
    assert_eq!(plan["runnable"], true);
}

#[when("I load the archive")]
fn load_archive(world: &mut BddWorld) {
    let kappa = world.kappa.clone().expect("kappa");
    let output = run_cli(world, &["--json", "holo", "load", &kappa]);
    assert!(
        output.status.success(),
        "holo load failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[when(expr = "I run the archive with input {string}")]
fn run_archive(world: &mut BddWorld, input: String) {
    let output = run_with_input(world, &input);
    assert!(
        output.status.success(),
        "holo run failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    world.run_result = Some(serde_json::from_slice(&output.stdout).expect("parse run output"));
}

#[when(expr = "I run the compiled archive directly with input {string}")]
fn run_local_archive(world: &mut BddWorld, input: String) {
    let input_path = world
        .temporary
        .as_ref()
        .expect("scenario directory")
        .path()
        .join("direct-input.bin");
    std::fs::write(&input_path, input).expect("write input");
    let output = Command::new(env!("CARGO_BIN_EXE_hologram"))
        .arg("--json")
        .arg("run")
        .arg(world.output_path.as_ref().expect("compiled archive"))
        .arg("--input")
        .arg(input_path)
        .env("HOME", home_path(world))
        .output()
        .expect("run local archive");
    assert!(
        output.status.success(),
        "local holo run failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    world.run_result = Some(serde_json::from_slice(&output.stdout).expect("parse run output"));
}

#[when("I run the compiled archive without a development grant")]
fn run_local_archive_without_grant(world: &mut BddWorld) {
    world.command_output = Some(
        Command::new(env!("CARGO_BIN_EXE_hologram"))
            .arg("--json")
            .arg("run")
            .arg(world.output_path.as_ref().expect("compiled archive"))
            .env("HOME", home_path(world))
            .output()
            .expect("run local archive"),
    );
}

#[then("the run fails with an authorization-denied error")]
fn run_fails_authorization(world: &mut BddWorld) {
    let output = world.command_output.as_ref().expect("run output");
    assert!(!output.status.success(), "run must fail without a grant");
    let error: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("parse JSON error");
    assert_eq!(error["code"], "LIVE_AUTHORIZATION_DENIED");
}

#[when(expr = "I run the compiled archive with its development grant and input {string}")]
fn run_local_archive_with_grant(world: &mut BddWorld, input: String) {
    let input_path = world
        .temporary
        .as_ref()
        .expect("scenario directory")
        .path()
        .join("granted-input.bin");
    std::fs::write(&input_path, input).expect("write input");
    let output = Command::new(env!("CARGO_BIN_EXE_hologram"))
        .arg("--json")
        .arg("run")
        .arg(world.output_path.as_ref().expect("compiled archive"))
        .arg("--development-grant")
        .arg(world.development_grant.as_ref().expect("development grant"))
        .arg("--input")
        .arg(input_path)
        .env("HOME", home_path(world))
        .output()
        .expect("run local archive with grant");
    assert!(
        output.status.success(),
        "granted local run failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    world.run_result = Some(serde_json::from_slice(&output.stdout).expect("parse run output"));
}

#[then(expr = "the run output is {string}")]
fn run_output_is(world: &mut BddWorld, expected: String) {
    let result = world.run_result.as_ref().expect("run result");
    let bytes: Vec<u8> = result["outputs"][0]
        .as_array()
        .expect("first output")
        .iter()
        .map(|byte| u8::try_from(byte.as_u64().expect("byte")).expect("byte in range"))
        .collect();
    assert_eq!(bytes, expected.as_bytes());
}

#[then(expr = "the run reports allowed authorization from {string}")]
fn run_reports_authorization(world: &mut BddWorld, source: String) {
    let result = world.run_result.as_ref().expect("run result");
    assert_eq!(result["authorization"], "allowed");
    assert_eq!(result["grant_source"], source);
    assert!(
        result["requested_capabilities_kappa"]
            .as_str()
            .is_some_and(|value| value.starts_with("blake3:")),
        "run result must identify its capability request"
    );
    assert!(
        result["effective_grant_kappa"]
            .as_str()
            .is_some_and(|value| value.starts_with("blake3:")),
        "run result must identify its effective grant"
    );
}

#[then("the run completion is returned without an exit code")]
fn run_completion_is_returned(world: &mut BddWorld) {
    let result = world.run_result.as_ref().expect("run result");
    assert_eq!(result["completion"]["kind"], "returned");
    assert!(result["completion"].get("code").is_none());
}

#[then(expr = "the capability audit records {string} from {string} for principal {string}")]
fn capability_audit_records(
    world: &mut BddWorld,
    outcome: String,
    source: String,
    principal: String,
) {
    let rows = capability_audit_rows(world);
    assert!(
        rows.iter().any(|row| {
            row["operation"] == "holo.capability.authorize"
                && row["principal"] == principal
                && row["outcome"] == outcome
                && row["capability_decision"]["grant_source"] == source
        }),
        "missing {outcome}/{source}/{principal} capability audit row: {rows:?}"
    );
}

#[then("the capability audit contains no source document or payload data")]
fn capability_audit_is_non_secret(world: &mut BddWorld) {
    let path = home_path(world).join(".local/state/hologram/audit.jsonl");
    let encoded = std::fs::read_to_string(path).expect("read audit log");
    for forbidden in [
        "network_fetch",
        "storage_roots",
        "development-grant.json",
        "authorized",
        "resident grant",
    ] {
        assert!(!encoded.contains(forbidden), "audit leaked {forbidden}");
    }
}

#[when(expr = "I run the archive over HTTP with input {string}")]
fn run_archive_over_http(world: &mut BddWorld, input: String) {
    let config_path = home_path(world).join(".config/hologram/live.toml");
    let config: toml::Value =
        toml::from_str(&std::fs::read_to_string(config_path).expect("read configuration"))
            .expect("parse configuration");
    let endpoint = config["client"]["local_endpoint"]
        .as_str()
        .expect("local endpoint");
    let kappa = world.kappa.as_ref().expect("kappa");
    let payload = serde_json::json!({"inputs": [input.into_bytes()]}).to_string();
    let output = Command::new("curl")
        .args([
            "--fail-with-body",
            "--silent",
            "--show-error",
            "--request",
            "POST",
            "--header",
            "content-type: application/json",
            "--data",
            &payload,
            &format!("{endpoint}/api/v1/holo/{kappa}/run"),
        ])
        .output()
        .expect("run HTTP request");
    assert!(
        output.status.success(),
        "HTTP holo run failed: {} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    world.run_result = Some(serde_json::from_slice(&output.stdout).expect("HTTP run result"));
}

#[then("the archive appears in the resident list")]
fn resident_list_contains_archive(world: &mut BddWorld) {
    let kappa = world.kappa.clone().expect("kappa");
    let output = run_cli(world, &["--json", "holo", "resident"]);
    assert!(
        output.status.success(),
        "holo resident failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let resident: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("parse resident output");
    assert!(
        resident
            .as_array()
            .expect("resident list")
            .iter()
            .any(|entry| entry["kappa"].as_str() == Some(kappa.as_str())),
        "resident list does not contain {kappa}: {resident}"
    );
}

#[when("I unload the archive")]
fn unload_archive(world: &mut BddWorld) {
    let kappa = world.kappa.clone().expect("kappa");
    let output = run_cli(world, &["holo", "unload", &kappa]);
    assert!(
        output.status.success(),
        "holo unload failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[then("running the archive fails with a not-found error")]
fn run_after_unload_is_not_found(world: &mut BddWorld) {
    let output = run_with_input(world, "still here");
    assert!(!output.status.success(), "run unexpectedly succeeded");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("LIVE_NOT_FOUND"), "stderr: {stderr}");
}

fn run_with_input(world: &BddWorld, input: &str) -> Output {
    let kappa = world.kappa.clone().expect("kappa");
    let input_path = world
        .temporary
        .as_ref()
        .expect("scenario directory")
        .path()
        .join("input.bin");
    std::fs::write(&input_path, input).expect("write input");
    let input_path = input_path.to_string_lossy().into_owned();
    run_cli(
        world,
        &["--json", "holo", "run", &kappa, "--input", &input_path],
    )
}

#[when("I compile the application")]
fn compile_application(world: &mut BddWorld) {
    let destination = world
        .temporary
        .as_ref()
        .expect("scenario directory")
        .path()
        .join("view.holo");
    let output = Command::new(env!("CARGO_BIN_EXE_hologram"))
        .arg("compile")
        .arg(world.manifest.as_ref().expect("manifest"))
        .arg("--output")
        .arg(&destination)
        .output()
        .expect("run hologram compile");
    world.output_path = Some(destination);
    world.command_output = Some(output);
}

#[then("the compile command succeeds")]
fn compile_succeeds(world: &mut BddWorld) {
    let output = world.command_output.as_ref().expect("command output");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[then("the output is a valid self-contained .holo archive")]
fn output_is_self_contained(world: &mut BddWorld) {
    let path = world.output_path.as_ref().expect("output path");
    let bytes = std::fs::read(path).expect("read compiled archive");
    let inspection = inspect_bytes("bdd", "view.holo", &bytes).expect("inspect archive");
    assert!(inspection.footer_verified);
    assert!(inspection
        .sections
        .iter()
        .any(|section| section.kind == "AppManifest"));
    assert!(inspection
        .sections
        .iter()
        .any(|section| section.kind == "ContentBlob"));
    assert!(inspection.directory_embedded);
    assert_eq!(
        inspection
            .directory
            .expect("queryable application directory")
            .layers
            .len(),
        1
    );
}

#[given("a fresh Hologram home")]
fn fresh_home(world: &mut BddWorld) {
    world.home = Some(tempfile::tempdir().expect("create home directory"));
}

#[given("an initialized configuration on a test port")]
fn initialized_config(world: &mut BddWorld) {
    let output = run_cli(world, &["init"]);
    assert!(
        output.status.success(),
        "init failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let config = home_path(world).join(".config/hologram/live.toml");
    let source = std::fs::read_to_string(&config).expect("read config");
    // A fresh port per scenario: `hologram stop` returns before the previous
    // scenario's daemon has fully drained, and a shared pid-derived port would
    // let a lingering listener answer this scenario's requests.
    let port = free_port();
    std::fs::write(&config, source.replace("11435", &port.to_string())).expect("write config");
}

#[given("the service uses the development grant")]
fn service_uses_development_grant(world: &mut BddWorld) {
    let config_path = home_path(world).join(".config/hologram/live.toml");
    let source = std::fs::read_to_string(&config_path).expect("read config");
    let mut config: toml::Value = toml::from_str(&source).expect("parse config");
    let holo = config
        .as_table_mut()
        .expect("configuration table")
        .entry("holo")
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
    holo.as_table_mut()
        .expect("holo configuration table")
        .insert(
            "development_grant".to_owned(),
            toml::Value::String(
                world
                    .development_grant
                    .as_ref()
                    .expect("development grant")
                    .display()
                    .to_string(),
            ),
        );
    std::fs::write(
        config_path,
        toml::to_string_pretty(&config).expect("encode config"),
    )
    .expect("write config");
}

#[when("the service declares the imported archive as a resident application")]
fn declare_resident_application(world: &mut BddWorld) {
    let kappa = world.kappa.clone().expect("kappa");
    let config_path = home_path(world).join(".config/hologram/live.toml");
    let source = std::fs::read_to_string(&config_path).expect("read config");
    let mut config: toml::Value = toml::from_str(&source).expect("parse config");
    let holo = config
        .as_table_mut()
        .expect("configuration table")
        .entry("holo")
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
    let resident = holo
        .as_table_mut()
        .expect("holo configuration table")
        .entry("resident")
        .or_insert_with(|| toml::Value::Array(Vec::new()));
    let mut entry = toml::map::Map::new();
    entry.insert("kappa".to_owned(), toml::Value::String(kappa));
    resident
        .as_array_mut()
        .expect("resident declaration array")
        .push(toml::Value::Table(entry));
    std::fs::write(
        config_path,
        toml::to_string_pretty(&config).expect("encode config"),
    )
    .expect("write config");
}

#[when("I restart the local service")]
fn restart_service(world: &mut BddWorld) {
    let output = run_cli(world, &["restart"]);
    assert!(
        output.status.success(),
        "restart failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .expect("bind ephemeral port")
        .local_addr()
        .expect("local address")
        .port()
}

#[when(expr = "I create a conversation titled {string}")]
fn create_conversation(world: &mut BddWorld, title: String) {
    let output = run_cli(world, &["--json", "history", "new", &title]);
    assert!(
        output.status.success(),
        "history new failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let record: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("parse conversation");
    world.conversation_id = Some(record["id"].as_str().expect("conversation id").to_owned());
}

#[when(expr = "I send {string} to the conversation")]
fn send_message(world: &mut BddWorld, message: String) {
    let id = world.conversation_id.clone().expect("conversation id");
    let output = run_cli(world, &["--json", "chat", "send", &id, &message]);
    world.sent_message = Some(message);
    world.command_output = Some(output);
}

#[then("the assistant response echoes the message")]
fn assistant_echoes(world: &mut BddWorld) {
    let conversation = conversation_output(world);
    let messages = conversation["messages"].as_array().expect("messages");
    let message = world.sent_message.as_ref().expect("sent message");
    let assistant = messages
        .iter()
        .find(|entry| entry["role"] == "assistant")
        .expect("assistant message");
    assert_eq!(assistant["content"].as_str(), Some(message.as_str()));
}

#[then("both sides of the exchange are recorded")]
fn exchange_recorded(world: &mut BddWorld) {
    let conversation = conversation_output(world);
    let messages = conversation["messages"].as_array().expect("messages");
    let message = world.sent_message.as_ref().expect("sent message");
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["role"].as_str(), Some("user"));
    assert_eq!(messages[0]["content"].as_str(), Some(message.as_str()));
    assert_eq!(messages[1]["role"].as_str(), Some("assistant"));
}

#[then("I stop the local service")]
fn stop_service(world: &mut BddWorld) {
    let output = run_cli(world, &["stop"]);
    assert!(
        output.status.success(),
        "stop failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[given("a fake weightc engine with resident sessions")]
fn fake_weightc_resident(world: &mut BddWorld) {
    world.temporary = Some(tempfile::tempdir().expect("create scenario directory"));
    let root = world
        .temporary
        .as_ref()
        .expect("scenario directory")
        .path()
        .to_path_buf();
    let artifact = root.join("tiny.wcpu");
    std::fs::create_dir_all(&artifact).expect("artifact dir");
    std::fs::write(artifact.join("manifest.json"), b"{}").expect("manifest");

    // Import with the stock echo config, then restart the daemon onto the
    // fake weightc engine: the daemon reads its config at startup.
    let output = run_cli(
        world,
        &["--json", "models", "import", &artifact.to_string_lossy()],
    );
    assert!(
        output.status.success(),
        "models import failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let record: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("parse import output");
    let model_id = record["id"].as_str().expect("model id").to_owned();
    stop_service(world);

    let fake = root.join("weightc");
    std::fs::copy(
        workspace_root().join("features/fixtures/fake-weightc/weightc"),
        &fake,
    )
    .expect("copy fake weightc");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755))
            .expect("chmod fake weightc");
    }

    let config_path = home_path(world).join(".config/hologram/live.toml");
    let source = std::fs::read_to_string(&config_path).expect("read config");
    let mut value: toml::Value = toml::from_str(&source).expect("parse config");
    value["inference"]["engine"] = toml::Value::String("weightc".to_owned());
    value["inference"]["weightc_path"] = toml::Value::String(fake.to_string_lossy().into_owned());
    value["inference"]["default_model"] = toml::Value::String(model_id.clone());
    value["inference"]["resident_sessions"] = toml::Value::Boolean(true);
    value["inference"]["max_resident_sessions"] = toml::Value::Integer(2);
    std::fs::write(
        &config_path,
        toml::to_string_pretty(&value).expect("encode config"),
    )
    .expect("write config");
    world.model_id = Some(model_id);
}

#[then(expr = "the assistant response is {string}")]
fn assistant_response_is(world: &mut BddWorld, expected: String) {
    let conversation = conversation_output(world);
    let messages = conversation["messages"].as_array().expect("messages");
    let assistant = messages
        .iter()
        .rev()
        .find(|entry| entry["role"] == "assistant")
        .expect("assistant message");
    assert_eq!(assistant["content"].as_str(), Some(expected.as_str()));
}

#[then("the fake engine served both turns on one resident process")]
fn fake_engine_served_one_process(world: &mut BddWorld) {
    let model_id = world.model_id.as_ref().expect("model id");
    let digest = model_id.strip_prefix("blake3:").unwrap_or(model_id);
    let log_path = home_path(world)
        .join(".local/share/hologram/models")
        .join(digest)
        .join("session.log");
    let log = std::fs::read_to_string(&log_path).expect("read session log");
    let starts: Vec<&str> = log
        .lines()
        .filter(|line| line.starts_with("start "))
        .collect();
    let turns: Vec<&str> = log
        .lines()
        .filter(|line| line.starts_with("turn "))
        .collect();
    assert_eq!(starts.len(), 1, "one resident process expected: {log}");
    assert_eq!(turns.len(), 2, "two turns expected: {log}");
    assert!(turns[0].contains("first turn"), "log: {log}");
    assert!(turns[1].contains("second turn"), "log: {log}");
}

#[given("the echo example plugin is enabled in the configuration")]
fn enable_echo_plugin(world: &mut BddWorld) {
    let binary = build_echo_example();
    let digest = {
        use sha2::Digest;
        hologram_live::util::hex(&sha2::Sha256::digest(
            std::fs::read(&binary).expect("read plugin binary"),
        ))
    };
    let config_path = home_path(world).join(".config/hologram/live.toml");
    let source = std::fs::read_to_string(&config_path).expect("read config");
    let mut value: toml::Value = toml::from_str(&source).expect("parse config");
    // macOS limits UDS paths to 104 bytes; the default state dir under a
    // tempdir home would overflow once the plugin socket name is appended.
    value["paths"]["state_dir"] =
        toml::Value::String(home_path(world).join(".st").to_string_lossy().into_owned());
    value["plugins"]["enabled"] = toml::Value::Boolean(true);
    let mut module = toml::map::Map::new();
    module.insert("id".to_owned(), "dev.hologram.examples.echo".into());
    module.insert(
        "path".to_owned(),
        binary.to_string_lossy().into_owned().into(),
    );
    module.insert("sha256".to_owned(), digest.into());
    value["plugins"]["modules"] = toml::Value::Array(vec![toml::Value::Table(module)]);
    std::fs::write(
        &config_path,
        toml::to_string_pretty(&value).expect("encode config"),
    )
    .expect("write config");
}

#[when("I list plugins")]
fn list_plugins(world: &mut BddWorld) {
    let output = run_cli(world, &["--json", "plugins", "list"]);
    assert!(
        output.status.success(),
        "plugins list failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    world.plugin_list = Some(serde_json::from_slice(&output.stdout).expect("parse plugin list"));
}

#[then("the plugin list contains the echo plugin")]
fn plugin_list_contains_echo(world: &mut BddWorld) {
    let list = world.plugin_list.as_ref().expect("plugin list");
    let entries = list.as_array().expect("plugin list array");
    let entry = entries
        .iter()
        .find(|entry| entry["id"] == "dev.hologram.examples.echo")
        .expect("echo plugin is listed");
    assert_eq!(entry["running"], true);
    assert!(entry["operations"]
        .as_array()
        .expect("operations")
        .iter()
        .any(|operation| operation == "echo.ping"));
}

#[when(expr = "I call plugin {string} operation {string} with payload {string}")]
fn call_plugin(world: &mut BddWorld, plugin: String, operation: String, payload: String) {
    let output = run_cli(world, &["plugins", "call", &plugin, &operation, &payload]);
    world.command_output = Some(output);
}

#[then(expr = "the plugin response is {string}")]
fn plugin_response_is(world: &mut BddWorld, expected: String) {
    let output = world.command_output.as_ref().expect("command output");
    assert!(
        output.status.success(),
        "plugins call failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let actual: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("parse plugin response");
    let expected: serde_json::Value = serde_json::from_str(&expected).expect("parse expected");
    assert_eq!(actual, expected);
}

#[then(expr = "the plugin call fails with code {string}")]
fn plugin_call_fails_with(world: &mut BddWorld, code: String) {
    let output = world.command_output.as_ref().expect("command output");
    assert!(
        !output.status.success(),
        "plugins call unexpectedly succeeded"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(&code), "stderr: {stderr}");
}

/// The fixture plugin is a cargo example; `CARGO_BIN_EXE_*` only covers bins,
/// so build it explicitly. The real HOME is inherited so cargo keeps its
/// registry cache.
fn build_echo_example() -> PathBuf {
    let root = workspace_root();
    let status = Command::new(env!("CARGO"))
        .args(["build", "--locked", "--example", "echo-plugin"])
        .current_dir(&root)
        .status()
        .expect("build echo-plugin example");
    assert!(status.success(), "cargo build --example echo-plugin failed");
    std::env::var_os("CARGO_TARGET_DIR")
        .map_or_else(|| root.join("target"), PathBuf::from)
        .join("debug/examples/echo-plugin")
}

fn run_cli(world: &BddWorld, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_hologram"))
        .args(args)
        .env("HOME", home_path(world))
        .output()
        .expect("run hologram CLI")
}

fn home_path(world: &BddWorld) -> PathBuf {
    world
        .home
        .as_ref()
        .expect("home directory")
        .path()
        .to_path_buf()
}

fn capability_audit_rows(world: &BddWorld) -> Vec<serde_json::Value> {
    let path = home_path(world).join(".local/state/hologram/audit.jsonl");
    std::fs::read_to_string(path)
        .expect("read audit log")
        .lines()
        .map(|row| serde_json::from_str(row).expect("audit JSON row"))
        .collect()
}

fn conversation_output(world: &BddWorld) -> serde_json::Value {
    let output = world.command_output.as_ref().expect("command output");
    assert!(
        output.status.success(),
        "command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("parse conversation output")
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

#[tokio::main]
async fn main() {
    BddWorld::cucumber()
        .fail_on_skipped_with(|feature, _rule, scenario| {
            feature
                .tags
                .iter()
                .chain(scenario.tags.iter())
                .any(|tag| tag.trim_start_matches('@') == "status:enforced")
        })
        .run_and_exit(concat!(env!("CARGO_MANIFEST_DIR"), "/features/suites"))
        .await;
}
