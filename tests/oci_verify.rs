#![cfg(feature = "oci")]
//! `hologram oci verify` (plan P8 T3, SC-009): every flipped byte is named,
//! with the repository and tags that reach it; a clean store reports nothing;
//! the command exits 1 on damage and changes nothing.

use hologram_live::oci_store::{
    Digest, LinkKind, ManifestPlan, OciStore, OpenOptions, Reference, RepoName,
};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

const MANIFEST_TYPE: &str = "application/vnd.oci.image.manifest.v1+json";

fn options() -> OpenOptions {
    OpenOptions {
        create: true,
        upload_max_age: Duration::from_hours(1),
    }
}

fn repo(name: &str) -> RepoName {
    RepoName::parse(name).expect("repository name")
}

fn push_blob(store: &OciStore, name: &str, bytes: &[u8]) -> Digest {
    let id = store.upload_begin(&repo(name)).expect("begin");
    store.upload_append(&id, 0, bytes).expect("append");
    store
        .upload_finish(&id, &Digest::sha256_of(bytes))
        .expect("finish")
}

/// `layers` blobs of 256 bytes in `team/app`, ten to a manifest, each
/// manifest tagged `t<n>`. Returns each layer's digest and its tag.
fn fill(store: &OciStore, layers: usize) -> Vec<(Digest, String)> {
    let mut out = Vec::new();
    for group in 0..layers.div_ceil(10) {
        let tag = format!("t{group}");
        let digests: Vec<Digest> = (group * 10..(group * 10 + 10).min(layers))
            .map(|n| {
                let mut bytes = vec![0_u8; 256];
                bytes[..8].copy_from_slice(&(n as u64).to_le_bytes());
                push_blob(store, "team/app", &bytes)
            })
            .collect();
        let layers_json: Vec<String> = digests
            .iter()
            .map(|digest| format!(r#"{{"digest":"{digest}"}}"#))
            .collect();
        let manifest = format!(
            r#"{{"schemaVersion":2,"mediaType":"{MANIFEST_TYPE}","layers":[{}]}}"#,
            layers_json.join(",")
        );
        let plan = ManifestPlan {
            kind: LinkKind::Manifest,
            must_exist: digests.clone(),
            subject: None,
        };
        store
            .manifest_put(
                &repo("team/app"),
                &Reference::parse(&tag).expect("tag"),
                MANIFEST_TYPE,
                manifest.as_bytes(),
                &plan,
            )
            .expect("manifest");
        out.extend(digests.into_iter().map(|digest| (digest, tag.clone())));
    }
    out
}

/// The file a blob is kept in (kappa-core's `blob_path_for`).
fn blob_file(root: &Path, digest: &Digest) -> PathBuf {
    let (algorithm, hex) = digest.as_str().split_once(':').expect("algorithm:hex");
    root.join("kappa/blobs")
        .join(algorithm)
        .join(&hex[..2])
        .join(&hex[2..4])
        .join(hex)
}

fn flip_one_byte(file: &Path) {
    let mut permissions = std::fs::metadata(file).expect("blob").permissions();
    #[allow(
        clippy::permissions_set_readonly_false,
        reason = "the test damages the file on purpose"
    )]
    permissions.set_readonly(false);
    std::fs::set_permissions(file, permissions).expect("writable");
    let mut bytes = std::fs::read(file).expect("read");
    bytes[100] ^= 0x01;
    std::fs::write(file, bytes).expect("write");
}

#[test]
fn names_every_flipped_byte_with_its_repository_and_tag() {
    let dir = tempfile::tempdir().expect("tempdir");
    let flipped: BTreeSet<(String, String)> = {
        let store = OciStore::open(dir.path(), options()).expect("open");
        let layers = fill(&store, 1000);
        layers
            .into_iter()
            .enumerate()
            .filter(|(n, _)| n % 10 == 3)
            .map(|(_, (digest, tag))| {
                flip_one_byte(&blob_file(dir.path(), &digest));
                (digest.as_str().to_owned(), tag)
            })
            .collect()
    };
    assert_eq!(flipped.len(), 100);
    let store = OciStore::open(dir.path(), options()).expect("reopen");
    let report = store.verify(&mut |_, _| {}).expect("verify");
    // Every object the store holds: the layers, the manifests, and the Kappa
    // store's own records, each hashed against its own address.
    assert!(report.checked >= 1100, "{} objects", report.checked);
    let named: BTreeSet<(String, String)> = report
        .damaged
        .iter()
        .map(|damaged| {
            assert!(
                damaged.reason.starts_with("hash mismatch"),
                "{}",
                damaged.reason
            );
            assert_eq!(damaged.repositories.len(), 1, "{damaged:?}");
            let reach = &damaged.repositories[0];
            assert_eq!(reach.name, "team/app");
            assert_eq!(reach.tags.len(), 1, "{damaged:?}");
            (damaged.digest.clone(), reach.tags[0].clone())
        })
        .collect();
    assert_eq!(
        named, flipped,
        "exactly the flipped blobs, each with its tag"
    );
}

#[test]
fn a_clean_store_reports_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = OciStore::open(dir.path(), options()).expect("open");
    fill(&store, 30);
    let mut seen = Vec::new();
    let report = store
        .verify(&mut |checked, total| seen.push((checked, total)))
        .expect("verify");
    assert!(report.damaged.is_empty(), "{report:?}");
    assert!(report.checked >= 33, "30 layers and 3 manifests at least");
    assert_eq!(
        seen.last(),
        Some(&(report.checked, report.checked)),
        "progress reaches the total"
    );
}

/// The command, as an operator runs it: `--json`, exit 1 on damage, and the
/// volume left as it was.
#[test]
fn the_command_reports_damage_and_exits_1_changing_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let volume = dir.path().join("volume");
    let damaged = {
        let store = OciStore::open(&volume, options()).expect("open");
        let layers = fill(&store, 10);
        let (digest, _) = &layers[4];
        flip_one_byte(&blob_file(&volume, digest));
        digest.as_str().to_owned()
    };
    let config = dir.path().join("config.yml");
    std::fs::write(
        &config,
        format!(
            "version: 0.1\nstorage:\n  filesystem:\n    rootdirectory: {}\n",
            volume.display().to_string().replace('\\', "/")
        ),
    )
    .expect("config.yml");
    let run = |json: bool| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_hologram"));
        if json {
            command.arg("--json");
        }
        command
            .args(["oci", "verify", "--registry-config"])
            .arg(&config)
            .env("HOME", dir.path())
            .env("USERPROFILE", dir.path())
            .env("HOLOGRAM_CONFIG_DIR", dir.path().join("config"))
            .output()
            .expect("hologram oci verify")
    };
    let before = std::fs::read(blob_file(
        &volume,
        &Digest::parse(&damaged).expect("digest"),
    ))
    .expect("blob");
    let output = run(true);
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(report["damaged"][0]["digest"], damaged);
    assert_eq!(report["damaged"][0]["repositories"][0]["tags"][0], "t0");
    let after = std::fs::read(blob_file(
        &volume,
        &Digest::parse(&damaged).expect("digest"),
    ))
    .expect("blob");
    assert_eq!(before, after, "verify changes nothing");
    let text = run(false);
    let stdout = String::from_utf8_lossy(&text.stdout);
    assert!(
        stdout.contains(&damaged) && stdout.contains("team/app: t0"),
        "{stdout}"
    );

    // A clean volume exits 0.
    let clean = dir.path().join("clean");
    fill(&OciStore::open(&clean, options()).expect("open"), 5);
    std::fs::write(
        &config,
        format!(
            "version: 0.1\nstorage:\n  filesystem:\n    rootdirectory: {}\n",
            clean.display().to_string().replace('\\', "/")
        ),
    )
    .expect("config.yml");
    let ok = run(false);
    assert!(
        ok.status.success(),
        "{}",
        String::from_utf8_lossy(&ok.stderr)
    );
    assert!(String::from_utf8_lossy(&ok.stdout).contains("every blob matches its digest"));
}

/// The paths the first test does not take: blobs stored under sha512 and
/// pushed by blake3, an index whose child is damaged, a damaged manifest, and
/// stray files in the tree that must not stop the run.
#[test]
fn every_address_kind_an_index_and_stray_files() {
    use sha2::Digest as _;
    let dir = tempfile::tempdir().expect("tempdir");
    let (sha512_blob, blake3_stored, child_layer, child_manifest) = {
        let store = OciStore::open(dir.path(), options()).expect("open");
        // sha512, kept as pushed.
        let bytes = b"a blob pushed by its sha512".to_vec();
        let id = store.upload_begin(&repo("odd/one")).expect("begin");
        store.upload_append(&id, 0, &bytes).expect("append");
        let claimed = Digest::parse(&format!("sha512:{}", hex(&sha2::Sha512::digest(&bytes))))
            .expect("sha512");
        let sha512_blob = store.upload_finish(&id, &claimed).expect("finish");
        // blake3: the store keeps it under sha256, with the blake3 alias.
        let bytes = b"a blob pushed by its blake3".to_vec();
        let id = store.upload_begin(&repo("odd/one")).expect("begin");
        store.upload_append(&id, 0, &bytes).expect("append");
        let blake3 = Digest::from_blake3(&blake3::hash(&bytes));
        store.upload_finish(&id, &blake3).expect("finish");
        let blake3_stored = store
            .blob_stat(&repo("odd/one"), &blake3)
            .expect("stat")
            .stored_as;
        // An index over two manifests, tagged; the second's layer is damaged.
        let a = push_blob(&store, "odd/multi", b"layer a");
        let b = push_blob(&store, "odd/multi", b"layer b");
        let mut children = Vec::new();
        for (tag, layer) in [("child-a", &a), ("child-b", &b)] {
            let body = format!(
                r#"{{"schemaVersion":2,"mediaType":"{MANIFEST_TYPE}","layers":[{{"digest":"{layer}"}}]}}"#
            );
            let plan = ManifestPlan {
                kind: LinkKind::Manifest,
                must_exist: vec![layer.clone()],
                subject: None,
            };
            let digest = store
                .manifest_put(
                    &repo("odd/multi"),
                    &Reference::parse(tag).expect("tag"),
                    MANIFEST_TYPE,
                    body.as_bytes(),
                    &plan,
                )
                .expect("manifest");
            children.push((digest, body.len()));
        }
        let index_type = "application/vnd.oci.image.index.v1+json";
        let entries: Vec<String> = children
            .iter()
            .map(|(digest, size)| {
                format!(r#"{{"mediaType":"{MANIFEST_TYPE}","digest":"{digest}","size":{size}}}"#)
            })
            .collect();
        let index = format!(
            r#"{{"schemaVersion":2,"mediaType":"{index_type}","manifests":[{}]}}"#,
            entries.join(",")
        );
        let plan = ManifestPlan {
            kind: LinkKind::Manifest,
            must_exist: children.iter().map(|(digest, _)| digest.clone()).collect(),
            subject: None,
        };
        store
            .manifest_put(
                &repo("odd/multi"),
                &Reference::parse("multi").expect("tag"),
                index_type,
                index.as_bytes(),
                &plan,
            )
            .expect("index");
        (sha512_blob, blake3_stored, b, children[0].0.clone())
    };
    for digest in [&sha512_blob, &blake3_stored, &child_layer, &child_manifest] {
        flip_one_byte_at(&blob_file(dir.path(), digest), 3);
    }
    // Stray files at the upper levels and in a leaf.
    let blobs = dir.path().join("kappa/blobs");
    std::fs::write(blobs.join("sha256/README.txt"), "notes").expect("stray");
    std::fs::write(blobs.join("sha256/ab-notes"), "notes").expect("stray");
    let leaf = blob_file(dir.path(), &child_layer);
    std::fs::write(leaf.with_file_name("not-an-address"), "junk").expect("stray");

    let store = OciStore::open(dir.path(), options()).expect("reopen");
    let report = store
        .verify(&mut |_, _| {})
        .expect("stray files do not stop the run");
    let by_digest = |digest: &Digest| {
        report
            .damaged
            .iter()
            .find(|damaged| damaged.digest == digest.as_str())
            .unwrap_or_else(|| panic!("{digest} not reported: {:?}", report.damaged))
    };
    assert_eq!(by_digest(&sha512_blob).kind, "mismatch", "the sha512 arm");
    assert_eq!(
        by_digest(&blake3_stored).kind,
        "mismatch",
        "a blake3 push, stored under sha256"
    );
    let layer = by_digest(&child_layer);
    assert_eq!(layer.repositories[0].name, "odd/multi");
    assert!(
        layer.repositories[0].tags.contains(&"multi".to_owned()),
        "reached through the index: {layer:?}"
    );
    assert!(
        layer.repositories[0].tags.contains(&"child-b".to_owned()),
        "{layer:?}"
    );
    let manifest = by_digest(&child_manifest);
    assert!(
        manifest.repositories[0]
            .tags
            .contains(&"child-a".to_owned()),
        "a damaged manifest is named: {manifest:?}"
    );
    assert!(
        manifest.repositories[0].tags.contains(&"multi".to_owned()),
        "{manifest:?}"
    );
    let junk = report
        .damaged
        .iter()
        .find(|damaged| damaged.digest.ends_with(":not-an-address"))
        .expect("the stray file in a leaf is reported");
    assert_eq!(junk.kind, "not-a-blob");
}

fn hex(bytes: &[u8]) -> String {
    hologram_live::util::hex(bytes)
}

fn flip_one_byte_at(file: &Path, at: usize) {
    let mut permissions = std::fs::metadata(file).expect("blob").permissions();
    #[allow(
        clippy::permissions_set_readonly_false,
        reason = "the test damages the file on purpose"
    )]
    permissions.set_readonly(false);
    std::fs::set_permissions(file, permissions).expect("writable");
    let mut bytes = std::fs::read(file).expect("read");
    bytes[at] ^= 0x01;
    std::fs::write(file, bytes).expect("write");
}

/// Beside a running server the volume is locked: the command says so, and
/// exits 5, not 1, so an operator's "exit 1 means damage" stays true.
#[test]
fn a_locked_volume_is_not_reported_as_damage() {
    let dir = tempfile::tempdir().expect("tempdir");
    let volume = dir.path().join("volume");
    let _held = OciStore::open(&volume, options()).expect("the running server's hold");
    let config = dir.path().join("config.yml");
    std::fs::write(
        &config,
        format!(
            "version: 0.1\nstorage:\n  filesystem:\n    rootdirectory: {}\n",
            volume.display().to_string().replace('\\', "/")
        ),
    )
    .expect("config.yml");
    let output = Command::new(env!("CARGO_BIN_EXE_hologram"))
        .args(["oci", "verify", "--registry-config"])
        .arg(&config)
        .env("HOME", dir.path())
        .env("USERPROFILE", dir.path())
        .env("HOLOGRAM_CONFIG_DIR", dir.path().join("config"))
        .output()
        .expect("hologram oci verify");
    assert_eq!(
        output.status.code(),
        Some(5),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("the registry is running on"));
}
