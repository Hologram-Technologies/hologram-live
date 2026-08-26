//! Opt-in end-to-end proofs for the pinned Python Component toolchain.
//!
//! This is ignored by the hermetic default suite because a cold run downloads
//! `componentize-py` through `uvx`. Run it with `just python-component-holo-demo`.

use hologram_live::compile::compile_manifest;
use hologram_live::holo::{HoloCatalog, HoloExecutor, HoloRuntime};
use hologram_live::store::ObjectStore;
use serde_json::Value;
use std::sync::Arc;

#[tokio::test]
#[ignore = "requires uvx and the pinned componentize-py tool"]
async fn dependency_free_python_runs_direct_and_resident() {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples/python-component-hello/hologram.json");
    let compiled = compile_manifest(&manifest).expect("compile Python component .holo");
    assert_compiled_provenance(&compiled, 0);

    let direct = HoloExecutor::default()
        .execute(&compiled.bytes, vec![b"Ada".to_vec()])
        .await
        .expect("run Python component directly");
    assert_response(&direct.outputs, "Ada");

    let directory = tempfile::tempdir().expect("catalog directory");
    let store = Arc::new(ObjectStore::open(directory.path()).expect("object store"));
    let catalog = Arc::new(HoloCatalog::new(store));
    let kappa = catalog
        .import("python-component-hello.holo".to_owned(), compiled.bytes)
        .expect("import Python component")
        .kappa;
    let runtime = HoloRuntime::new(catalog, 8);
    runtime.load(&kappa).await.expect("load Python component");
    let resident = runtime
        .run(&kappa, vec![b"Grace".to_vec()])
        .await
        .expect("run Python component resident");
    assert_response(&resident.outputs, "Grace");
    runtime
        .unload(&kappa)
        .await
        .expect("unload Python component");
}

#[tokio::test]
#[ignore = "requires uv and uvx plus the pinned component toolchain and wheel"]
async fn locked_pure_python_dependency_runs_direct_and_resident() {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples/python-component-dependency/hologram.json");
    let compiled = compile_manifest(&manifest).expect("compile dependency Python component .holo");
    assert_compiled_provenance(&compiled, 1);

    let direct = HoloExecutor::default()
        .execute(&compiled.bytes, vec![b"Ada".to_vec()])
        .await
        .expect("run dependency Python component directly");
    assert_dependency_response(&direct.outputs, "Ada");

    let directory = tempfile::tempdir().expect("catalog directory");
    let store = Arc::new(ObjectStore::open(directory.path()).expect("object store"));
    let catalog = Arc::new(HoloCatalog::new(store));
    let kappa = catalog
        .import(
            "python-component-dependency.holo".to_owned(),
            compiled.bytes,
        )
        .expect("import dependency Python component")
        .kappa;
    let runtime = HoloRuntime::new(catalog, 8);
    runtime
        .load(&kappa)
        .await
        .expect("load dependency Python component");
    let resident = runtime
        .run(&kappa, vec![b"Grace".to_vec()])
        .await
        .expect("run dependency Python component resident");
    assert_dependency_response(&resident.outputs, "Grace");
    runtime
        .unload(&kappa)
        .await
        .expect("unload dependency Python component");
}

#[tokio::test]
#[ignore = "requires uv and uvx plus the pinned component toolchain and wheel"]
async fn ambient_python_path_cannot_replace_locked_dependency() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest = root.join("examples/python-component-dependency/hologram.json");
    let temporary = tempfile::tempdir().expect("temporary compile directory");
    let poison = temporary.path().join("poison");
    std::fs::create_dir_all(&poison).expect("poison directory");
    std::fs::write(
        poison.join("six.py"),
        "__version__ = 'ambient-poison'\ndef ensure_text(value, encoding='utf-8'): return 'Poison'\n",
    )
    .expect("poison six module");
    let output = temporary.path().join("application.holo");

    let command = std::process::Command::new(env!("CARGO_BIN_EXE_hologram"))
        .arg("--json")
        .arg("compile")
        .arg(&manifest)
        .arg("--output")
        .arg(&output)
        .env("PYTHONPATH", &poison)
        .env("VIRTUAL_ENV", &poison)
        .output()
        .expect("start hologram compile");
    assert!(
        command.status.success(),
        "compile failed: {}",
        String::from_utf8_lossy(&command.stderr)
    );
    let report: Value = serde_json::from_slice(&command.stdout).expect("JSON compile report");
    assert_eq!(report["build_provenance"]["schema_version"], 1);
    assert_eq!(
        report["build_provenance"]["layers"][0]["source"]["dependencies"][0]["name"],
        "six"
    );

    let archive = std::fs::read(&output).expect("compiled archive");
    let direct = HoloExecutor::default()
        .execute(&archive, vec![b"Ada".to_vec()])
        .await
        .expect("run dependency Python component");
    assert_dependency_response(&direct.outputs, "Ada");
}

fn assert_compiled_provenance(
    compiled: &hologram_live::compile::CompiledHolo,
    dependency_count: usize,
) {
    assert_eq!(compiled.build_provenance.schema_version, 1);
    assert!(!compiled.build_provenance.canonical);
    let [layer] = compiled.build_provenance.layers.as_slice() else {
        panic!("expected one provenance layer");
    };
    assert_eq!(layer.layer_index, 0);
    assert_eq!(layer.language, "python");
    assert_eq!(layer.source.dependencies.len(), dependency_count);
    let distribution = layer
        .source
        .componentizer
        .distribution
        .as_ref()
        .expect("componentizer distribution");
    assert!(std::path::Path::new(distribution.url)
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("whl")));
    assert_eq!(distribution.sha256.len(), 64);
    assert_eq!(
        layer
            .source
            .componentizer_runner
            .as_ref()
            .map(|tool| tool.name),
        Some("uvx")
    );
    assert_eq!(
        layer
            .source
            .dependency_installer
            .as_ref()
            .map(|tool| tool.name),
        (dependency_count > 0).then_some("uv")
    );
    let output = layer.source.output.as_ref().expect("component output");
    assert_eq!(
        output.layer_kappa,
        hologram::space::address_bytes(
            hologram::archive::HoloLoader::from_bytes(&compiled.bytes)
                .expect("compiled archive")
                .into_plan()
                .expect("archive plan")
                .content_blobs()
                .expect("content blobs")
                .into_iter()
                .max_by_key(|(_, content)| content.len())
                .expect("component blob")
                .1
        )
        .to_string()
    );
    assert!(!layer.source.reproducibility.reproducible);
}

fn assert_response(outputs: &[Vec<u8>], expected_name: &str) {
    let [output] = outputs else {
        panic!("expected one Python output, got {}", outputs.len());
    };
    let value: Value = serde_json::from_slice(output).expect("Python JSON output");
    assert_eq!(value["name"], expected_name);
    assert_eq!(value["runtime"], "python-component");
    assert!(value["python"]
        .as_str()
        .is_some_and(|value| !value.is_empty()));
}

fn assert_dependency_response(outputs: &[Vec<u8>], expected_name: &str) {
    let [output] = outputs else {
        panic!("expected one Python output, got {}", outputs.len());
    };
    let value: Value = serde_json::from_slice(output).expect("Python JSON output");
    assert_eq!(value["name"], expected_name);
    assert_eq!(value["runtime"], "python-component");
    assert_eq!(value["dependency"], "six-1.17.0");
}
