#![cfg(feature = "oci")]
//! Integration tests for the store adapter. Run on Linux, macOS and Windows.

use hologram_live::oci_store::{OciStore, OciStoreError, OpenOptions};
use std::time::Duration;

fn options() -> OpenOptions {
    OpenOptions {
        create: true,
        upload_max_age: Duration::from_secs(7 * 24 * 3600),
    }
}

#[test]
fn a_new_directory_gets_layout_version_one() {
    let dir = tempfile::tempdir().expect("tempdir");
    drop(OciStore::open(dir.path(), options()).expect("open"));
    let marker =
        std::fs::read_to_string(dir.path().join("HOLOGRAM_REGISTRY_LAYOUT")).expect("marker");
    assert_eq!(marker.trim(), "1");
    assert!(dir.path().join("oci/links.redb").exists());
    assert!(dir.path().join("kappa/kappa.redb").exists());
}

#[test]
fn a_docker_registry_volume_is_refused_and_names_the_import_command() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("docker/registry/v2/repositories")).expect("mkdir");
    let error = OciStore::open(dir.path(), options()).expect_err("must refuse");
    let OciStoreError::Layout(message) = error else {
        panic!("wrong variant: {error:?}")
    };
    assert!(message.contains("hologram oci import"), "{message}");
    assert!(
        !dir.path().join("oci").exists(),
        "a refused volume is not touched"
    );
}

#[test]
fn a_newer_layout_is_refused_by_number() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("HOLOGRAM_REGISTRY_LAYOUT"), "2\n").expect("write");
    let error = OciStore::open(dir.path(), options()).expect_err("must refuse");
    assert!(matches!(error, OciStoreError::Layout(m) if m.contains('2') && m.contains('1')));
}

#[test]
fn a_second_opener_gets_locked() {
    let dir = tempfile::tempdir().expect("tempdir");
    let _first = OciStore::open(dir.path(), options()).expect("open");
    assert!(matches!(
        OciStore::open(dir.path(), options()),
        Err(OciStoreError::Locked)
    ));
}
