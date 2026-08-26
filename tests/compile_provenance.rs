use serde_json::Value;
use std::process::Command;

#[test]
fn compile_check_reports_versioned_noncanonical_python_provenance() {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples/python-component-dependency/hologram.json");
    let output = Command::new(env!("CARGO_BIN_EXE_hologram"))
        .arg("--json")
        .arg("compile")
        .arg(&manifest)
        .arg("--check")
        .output()
        .expect("run compile check");
    assert!(
        output.status.success(),
        "compile check failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).expect("JSON compile report");
    let provenance = &report["build_provenance"];
    assert_eq!(provenance["schema_version"], 1);
    assert_eq!(provenance["canonical"], false);
    assert_eq!(provenance["layers"][0]["layer_index"], 0);
    assert_eq!(provenance["layers"][0]["language"], "python");
    let source = &provenance["layers"][0]["source"];
    assert_eq!(source["profile"], "wasi-component");
    assert_eq!(source["guest_contract"], "hologram:guest/component@1");
    assert_eq!(source["target_abi"], "wasm32-wasip2-component");
    assert_eq!(source["runtime"]["version"], "3.14.0");
    assert_eq!(source["componentizer"]["version"], "0.25.0");
    assert!(source.get("componentizer_runner").is_none());
    assert!(source.get("dependency_installer").is_none());
    assert!(source.get("output").is_none());
    assert_eq!(source["dependencies"][0]["name"], "six");
    assert_eq!(source["dependencies"][0]["version"], "1.17.0");
    assert!(source["dependencies"][0]["wheel_url"]
        .as_str()
        .is_some_and(|url| url.starts_with("https://")));
    assert_eq!(
        source["dependencies"][0]["sha256"].as_str().map(str::len),
        Some(64)
    );
    assert_eq!(source["reproducibility"]["reproducible"], false);
    assert!(source["reproducibility"]["blocker"]
        .as_str()
        .is_some_and(|blocker| blocker.contains("no deterministic seed control")));
}
