use serde_json::{json, Value};
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
    assert!(source["componentizer"]["distribution"]["url"]
        .as_str()
        .is_some_and(|url| {
            url.starts_with("https://github.com/Hologram-Technologies/hologram-live/releases/download/componentizer-v0.25.0-hologram.1/")
                && std::path::Path::new(url)
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("whl"))
        }));
    assert_eq!(
        source["componentizer"]["distribution"]["sha256"]
            .as_str()
            .map(str::len),
        Some(64)
    );
    let patch_set = &source["componentizer"]["patch_set"];
    assert_eq!(patch_set["release_tag"], "componentizer-v0.25.0-hologram.1");
    assert_eq!(
        patch_set["release_url"],
        "https://github.com/Hologram-Technologies/hologram-live/releases/tag/componentizer-v0.25.0-hologram.1"
    );
    assert_eq!(
        patch_set["manifest_url"],
        "https://github.com/Hologram-Technologies/hologram-live/releases/download/componentizer-v0.25.0-hologram.1/PATCHSET.sha256"
    );
    assert_eq!(
        patch_set["manifest_sha256"],
        "25e19905ce9a12c341741e1b5754307e1d6e07bdf3a1f7bcaa7739595dc82167"
    );
    assert_eq!(
        patch_set["determinism_contract"],
        "hologram:componentizer/preinitialization-determinism@1"
    );
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
        .is_some_and(|blocker| blocker.contains("two independent clean builds")));
}

#[test]
fn rootfs_compile_check_reports_planned_provenance_without_docker() {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples/python-numpy-pandas/hologram.json");
    let output = Command::new(env!("CARGO_BIN_EXE_hologram"))
        .arg("--json")
        .arg("compile")
        .arg(&manifest)
        .arg("--check")
        .output()
        .expect("run rootfs compile check");
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
    assert_eq!(source["profile"], "rootfs");
    assert!(source["target_platform"]
        .as_str()
        .is_some_and(|platform| platform == "linux/arm64" || platform == "linux/amd64"));
    assert_eq!(source["base_image"]["reference"], "python:3.12-slim");
    assert_eq!(source["base_image"]["digest_pinned"], false);
    assert!(source["base_image"].get("resolved_reference").is_none());
    assert!(source["base_image"].get("observed_image_id").is_none());
    assert_eq!(source["dependency_installer"]["name"], "uv");
    assert_eq!(source["dependency_installer"]["version"], "0.11.8");
    assert_eq!(source["builder"]["name"], "docker");
    assert_eq!(
        source["builder"]["archive_format"],
        "normalized-docker-archive-v1"
    );
    assert_eq!(source["builder"]["source_date_epoch"], 0);
    assert_eq!(source["builder"]["cache_disabled"], false);
    assert!(source["builder"].get("client_version").is_none());
    assert!(source["builder"].get("server_version").is_none());
    assert!(source.get("output").is_none());
    assert_eq!(source["inputs"].as_array().map(Vec::len), Some(3));
    assert!(source["inputs"].as_array().is_some_and(|inputs| inputs
        .iter()
        .all(|input| input["sha256"]
            .as_str()
            .is_some_and(|hash| hash.len() == 64))));
    assert_eq!(source["reproducibility"]["reproducible"], false);
    assert!(source["reproducibility"]["blocker"]
        .as_str()
        .is_some_and(|blocker| blocker.contains("not resolved until compilation")
            && blocker.contains("immutable digest")));
}

#[test]
fn compile_check_rejects_the_no_build_cache_execution_option() {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples/python-numpy-pandas/hologram.json");
    let output = Command::new(env!("CARGO_BIN_EXE_hologram"))
        .arg("--json")
        .arg("compile")
        .arg(&manifest)
        .arg("--check")
        .arg("--no-build-cache")
        .output()
        .expect("run compile check");
    assert!(!output.status.success());
    let error: Value = serde_json::from_slice(&output.stdout).expect("JSON error report");
    assert_eq!(error["code"], "LIVE_CONFIG_INVALID");
    assert!(error["message"]
        .as_str()
        .is_some_and(|message| message.contains("--no-build-cache")));
}

#[test]
fn rootfs_report_comparator_groups_clean_replicas_by_target() {
    let directory = tempfile::tempdir().expect("temporary reports");
    let targets = ["linux/amd64", "linux/amd64", "linux/arm64", "linux/arm64"];
    for (index, target) in targets.iter().enumerate() {
        let identity = if *target == "linux/amd64" {
            "amd64-identity"
        } else {
            "arm64-identity"
        };
        let report = json!({
            "status": "ok",
            "target_platform": target,
            "build_host": {"os": "linux", "arch": target},
            "builder": {"cache_disabled": true},
            "no_build_cache": true,
            "equal": true,
            "identities": {"image_id": identity},
        });
        std::fs::write(
            directory.path().join(format!("report-{index}.json")),
            serde_json::to_vec(&report).expect("encode report"),
        )
        .expect("write report");
    }

    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("scripts/compare-python-rootfs-reports.py");
    let mut command = Command::new("python3");
    command
        .arg(script)
        .arg("--expected-replicas")
        .arg("2")
        .arg("--expected-targets")
        .arg("2");
    for index in 0..targets.len() {
        command.arg(directory.path().join(format!("report-{index}.json")));
    }
    let output = command.output().expect("compare reports");
    assert!(
        output.status.success(),
        "comparison failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).expect("comparison JSON");
    assert_eq!(result["status"], "ok");
    assert_eq!(result["targets"].as_array().map(Vec::len), Some(2));
    assert!(result["targets"]
        .as_array()
        .is_some_and(|targets| targets.iter().all(|target| target["equal"] == true)));
}
