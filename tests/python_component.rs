//! Opt-in end-to-end proof for the pinned Python Component toolchain.
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
