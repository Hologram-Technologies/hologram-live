#![cfg(feature = "oci")]
//! `garbage-collect` (plan P8 T1, SC-008): what the reference sweeps, the
//! store's own records never swept, and a property test: after every collect
//! over random pushes, tags, untags and deletes across three repositories
//! sharing layers, every tag still pulls completely.

use hologram_live::oci_store::{
    Digest, GcOptions, LinkKind, ManifestPlan, OciStore, OpenOptions, Reference, RepoName,
    SubjectPlan,
};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;
use std::time::Duration;

const MANIFEST: &str = "application/vnd.oci.image.manifest.v1+json";
const INDEX: &str = "application/vnd.oci.image.index.v1+json";

fn open(dir: &Path) -> OciStore {
    OciStore::open(
        dir,
        OpenOptions {
            create: true,
            upload_max_age: Duration::from_hours(1),
        },
    )
    .expect("open")
}

fn repo(name: &str) -> RepoName {
    RepoName::parse(name).expect("repository name")
}

fn blob(store: &OciStore, name: &str, bytes: &[u8]) -> Digest {
    let id = store.upload_begin(&repo(name)).expect("begin");
    store.upload_append(&id, 0, bytes).expect("append");
    store
        .upload_finish(&id, &Digest::sha256_of(bytes))
        .expect("finish")
}

/// A manifest over `layers` in `name`, tagged `tag`, or pushed by digest.
fn manifest(
    store: &OciStore,
    name: &str,
    tag: Option<&str>,
    layers: &[&Digest],
    subject: Option<&Digest>,
) -> Digest {
    let layers_json: Vec<String> = layers
        .iter()
        .map(|digest| format!(r#"{{"digest":"{digest}"}}"#))
        .collect();
    let subject_json = subject.map_or(String::new(), |digest| {
        format!(r#","subject":{{"mediaType":"{MANIFEST}","digest":"{digest}","size":1}}"#)
    });
    let body = format!(
        r#"{{"schemaVersion":2,"mediaType":"{MANIFEST}","layers":[{}]{subject_json}}}"#,
        layers_json.join(",")
    );
    let digest = Digest::sha256_of(body.as_bytes());
    let plan = ManifestPlan {
        kind: LinkKind::Manifest,
        must_exist: layers.iter().map(|digest| (*digest).clone()).collect(),
        subject: subject.map(|subject| SubjectPlan {
            subject: subject.clone(),
            artifact_type: None,
            annotations: None,
        }),
    };
    let reference = tag.map_or_else(
        || Reference::Digest(digest.clone()),
        |tag| Reference::parse(tag).expect("tag"),
    );
    store
        .manifest_put(&repo(name), &reference, MANIFEST, body.as_bytes(), &plan)
        .expect("manifest")
}

/// Every tag of `name` pulls completely: the manifest, and every blob it
/// names, byte for byte from the store.
fn every_tag_pulls(store: &OciStore, name: &str) {
    let Ok(tags) = store.tags_page(&repo(name), None, 1000) else {
        return;
    };
    for tag in tags {
        let manifest = store
            .manifest_get(&repo(name), &Reference::Tag(tag.clone()))
            .unwrap_or_else(|error| panic!("{name}:{} does not pull: {error}", tag.as_str()));
        let value: serde_json::Value = serde_json::from_slice(&manifest.bytes).expect("json");
        for layer in value["layers"].as_array().into_iter().flatten() {
            let digest = Digest::parse(layer["digest"].as_str().expect("digest")).expect("digest");
            let (_, mut reader) = store
                .blob_open(&repo(name), &digest)
                .unwrap_or_else(|error| panic!("{name}:{} layer {digest}: {error}", tag.as_str()));
            let mut bytes = Vec::new();
            reader.read_to_end(&mut bytes).expect("read");
            assert_eq!(
                Digest::sha256_of(&bytes),
                digest,
                "{name}:{} layer bytes",
                tag.as_str()
            );
        }
    }
}

fn files(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).expect("dir").flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                out.insert(
                    path.strip_prefix(root)
                        .expect("inside")
                        .display()
                        .to_string(),
                    std::fs::read(&path).unwrap_or_default(),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

fn collect(
    store: &OciStore,
    options: GcOptions,
) -> (hologram_live::oci_store::GcReport, Vec<String>) {
    let mut lines = Vec::new();
    let report = store
        .collect(options, &mut |line| lines.push(line))
        .expect("collect");
    (report, lines)
}

#[test]
fn the_reference_rule_orphans_go_untagged_manifests_stay_without_the_flag() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = open(dir.path());
    let (l1, l2, l3) = (
        blob(&store, "a/b", b"layer one"),
        blob(&store, "a/b", b"layer two"),
        blob(&store, "a/b", b"layer three"),
    );
    let orphan = blob(&store, "a/b", b"pushed, never in a manifest");
    manifest(&store, "a/b", Some("v1"), &[&l1, &l2], None);
    let untagged = manifest(&store, "a/b", None, &[&l3], None);
    let records_before: usize = files(&dir.path().join("kappa/blobs")).len();

    let (report, lines) = collect(&store, GcOptions::default());
    assert_eq!(report.swept_blobs, 1, "{lines:#?}");
    assert!(
        lines.contains(&format!("blob eligible for deletion: {orphan}")),
        "{lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line == &format!("a/b: marking manifest {untagged} ")),
        "the reference marks every manifest: {lines:#?}"
    );
    assert!(
        lines.iter().any(
            |line| line.contains("blobs marked, 1 blobs and 0 manifests eligible for deletion")
        ),
        "{lines:#?}"
    );
    // Only the orphan's file went: the store's own records all stayed.
    assert_eq!(
        files(&dir.path().join("kappa/blobs")).len(),
        records_before - 1
    );
    assert!(store.blob_stat(&repo("a/b"), &orphan).is_err());
    every_tag_pulls(&store, "a/b");
    assert!(
        store.blob_stat(&repo("a/b"), &l3).is_ok(),
        "the untagged manifest's layer stays"
    );

    let (report, lines) = collect(
        &store,
        GcOptions {
            dry_run: false,
            delete_untagged: true,
        },
    );
    assert_eq!(report.swept_manifests, 1, "{lines:#?}");
    assert!(
        lines.iter().any(|line| line.starts_with(&format!(
            "manifest eligible for deletion: {{a/b {untagged} ["
        ))),
        "{lines:#?}"
    );
    assert!(
        store.blob_stat(&repo("a/b"), &l3).is_err(),
        "its layer goes with it"
    );
    assert!(store
        .manifest_get(&repo("a/b"), &Reference::Digest(untagged))
        .is_err());
    every_tag_pulls(&store, "a/b");
}

#[test]
fn shared_layers_indexes_and_signatures_are_kept() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = open(dir.path());
    // One layer in two repositories; the first untags, the second still needs it.
    let shared = blob(&store, "one/app", b"shared layer");
    blob(&store, "two/app", b"shared layer");
    manifest(&store, "one/app", None, &[&shared], None);
    manifest(&store, "two/app", Some("v1"), &[&shared], None);
    // An index tagged over two untagged children.
    let (x, y) = (blob(&store, "idx/app", b"x"), blob(&store, "idx/app", b"y"));
    let child_x = manifest(&store, "idx/app", None, &[&x], None);
    let child_y = manifest(&store, "idx/app", None, &[&y], None);
    let index = format!(
        r#"{{"schemaVersion":2,"mediaType":"{INDEX}","manifests":[{{"mediaType":"{MANIFEST}","digest":"{child_x}","size":1}},{{"mediaType":"{MANIFEST}","digest":"{child_y}","size":1}}]}}"#
    );
    let plan = ManifestPlan {
        kind: LinkKind::Index,
        must_exist: vec![child_x.clone(), child_y.clone()],
        subject: None,
    };
    store
        .manifest_put(
            &repo("idx/app"),
            &Reference::parse("multi").expect("tag"),
            INDEX,
            index.as_bytes(),
            &plan,
        )
        .expect("index");
    // A signature: an untagged manifest whose subject is a tagged one.
    let signed_layer = blob(&store, "sig/app", b"signed");
    let signed = manifest(&store, "sig/app", Some("v1"), &[&signed_layer], None);
    let sig_blob = blob(&store, "sig/app", b"signature bytes");
    let signature = manifest(&store, "sig/app", None, &[&sig_blob], Some(&signed));

    let (_, lines) = collect(
        &store,
        GcOptions {
            dry_run: false,
            delete_untagged: true,
        },
    );
    for name in ["one/app", "two/app", "idx/app", "sig/app"] {
        every_tag_pulls(&store, name);
    }
    assert!(
        store.blob_stat(&repo("two/app"), &shared).is_ok(),
        "{lines:#?}"
    );
    for kept in [&child_x, &child_y] {
        assert!(
            store
                .manifest_get(&repo("idx/app"), &Reference::Digest(kept.clone()))
                .is_ok(),
            "an index keeps its children: {lines:#?}"
        );
    }
    assert!(
        store
            .manifest_get(&repo("sig/app"), &Reference::Digest(signature))
            .is_ok(),
        "a signature of a kept image stays: {lines:#?}"
    );
    assert!(store.blob_stat(&repo("sig/app"), &sig_blob).is_ok());
}

#[test]
fn a_dry_run_changes_nothing_and_says_what_it_would_do() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = open(dir.path());
    let orphan = blob(&store, "a/b", b"orphan");
    let kept = blob(&store, "a/b", b"kept");
    manifest(&store, "a/b", Some("v1"), &[&kept], None);
    manifest(&store, "a/b", None, &[&kept], None);
    drop(store);
    let before = files(dir.path());
    let store = open(dir.path());
    let (_, lines) = collect(
        &store,
        GcOptions {
            dry_run: true,
            delete_untagged: true,
        },
    );
    assert!(
        lines.contains(&format!("blob eligible for deletion: {orphan}")),
        "{lines:#?}"
    );
    drop(store);
    let after = files(dir.path());
    let blobs = |tree: &BTreeMap<String, Vec<u8>>| {
        tree.iter()
            .filter(|(path, _)| path.contains("blobs"))
            .map(|(p, b)| (p.clone(), b.clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(blobs(&before), blobs(&after), "no blob file changed");
    let store = open(dir.path());
    assert!(
        store.blob_stat(&repo("a/b"), &orphan).is_ok(),
        "the orphan is still linked"
    );
}

#[test]
fn a_manifest_that_cannot_be_read_stops_the_run_before_any_sweep() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = open(dir.path());
    let orphan = blob(&store, "a/b", b"orphan");
    let layer = blob(&store, "a/b", b"layer");
    let broken = manifest(&store, "a/b", Some("v1"), &[&layer], None);
    drop(store);
    // Damage the manifest's bytes on disk so it no longer parses.
    let (algorithm, hex) = broken.as_str().split_once(':').expect("digest");
    let path = dir
        .path()
        .join("kappa/blobs")
        .join(algorithm)
        .join(&hex[..2])
        .join(&hex[2..4])
        .join(hex);
    let mut permissions = std::fs::metadata(&path).expect("file").permissions();
    #[allow(
        clippy::permissions_set_readonly_false,
        reason = "the test damages the file on purpose"
    )]
    permissions.set_readonly(false);
    std::fs::set_permissions(&path, permissions).expect("writable");
    std::fs::write(&path, b"{ not json").expect("damage");
    let store = open(dir.path());
    let error = store
        .collect(GcOptions::default(), &mut |_| {})
        .expect_err("mark must stop");
    assert!(error.to_string().contains("nothing was swept"), "{error}");
    assert!(
        store.blob_stat(&repo("a/b"), &orphan).is_ok(),
        "nothing was swept"
    );
}

/// A small generator: no dependency, a fixed seed list plus one from the
/// clock, printed on failure.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

fn one_case(seed: u64) {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = open(dir.path());
    let mut rng = Rng(seed | 1);
    let repos = ["p/one", "p/two", "p/three"];
    for step in 0..24 {
        let name = repos[usize::try_from(rng.below(3)).expect("index")];
        match rng.below(8) {
            // Push an image of one to three layers from a small shared pool, tagged.
            0..=2 => {
                let count = 1 + rng.below(3);
                let layers: Vec<Digest> = (0..count)
                    .map(|_| blob(&store, name, format!("layer {}", rng.below(6)).as_bytes()))
                    .collect();
                let refs: Vec<&Digest> = layers.iter().collect();
                let tag = format!("t{}", rng.below(4));
                manifest(&store, name, Some(&tag), &refs, None);
            }
            // Push untagged.
            3 => {
                let layer = blob(&store, name, format!("layer {}", rng.below(6)).as_bytes());
                manifest(&store, name, None, &[&layer], None);
            }
            // Untag.
            4 => {
                if let Ok(tags) = store.tags_page(&repo(name), None, 10) {
                    if let Some(tag) = tags.first() {
                        store.tag_delete(&repo(name), tag).expect("untag");
                    }
                }
            }
            // An orphan blob.
            5 => {
                blob(&store, name, format!("orphan {step}").as_bytes());
            }
            // Collect, with or without the flag.
            6 => {
                store
                    .collect(GcOptions::default(), &mut |_| {})
                    .unwrap_or_else(|error| panic!("seed {seed}: {error}"));
                for name in repos {
                    every_tag_pulls(&store, name);
                }
            }
            _ => {
                store
                    .collect(
                        GcOptions {
                            dry_run: false,
                            delete_untagged: true,
                        },
                        &mut |_| {},
                    )
                    .unwrap_or_else(|error| panic!("seed {seed}: {error}"));
                for name in repos {
                    every_tag_pulls(&store, name);
                }
            }
        }
    }
    store
        .collect(
            GcOptions {
                dry_run: false,
                delete_untagged: true,
            },
            &mut |_| {},
        )
        .unwrap_or_else(|error| panic!("seed {seed}: {error}"));
    for name in repos {
        every_tag_pulls(&store, name);
    }
}

#[test]
fn after_every_collect_every_tag_pulls() {
    let cases: u64 = std::env::var("GC_PROPERTY_CASES")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(256);
    let clock = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(1, |elapsed| elapsed.as_secs());
    for case in 0..cases {
        let seed = if case + 1 == cases {
            clock
        } else {
            0x9E37_79B9_7F4A_7C15_u64.wrapping_mul(case + 1)
        };
        let outcome = std::panic::catch_unwind(|| one_case(seed));
        assert!(outcome.is_ok(), "property failed for seed {seed}");
    }
}

/// The command, as an operator runs it after stopping the registry.
#[test]
fn the_command_prints_the_references_lines_and_sweeps() {
    let dir = tempfile::tempdir().expect("tempdir");
    let volume = dir.path().join("volume");
    let orphan = {
        let store = open(&volume);
        let layer = blob(&store, "a/b", b"kept layer");
        manifest(&store, "a/b", Some("v1"), &[&layer], None);
        blob(&store, "a/b", b"orphan layer")
    };
    let config = dir.path().join("config.yml");
    let write_config = |root: &Path| {
        std::fs::write(
            &config,
            format!(
                "version: 0.1\nstorage:\n  filesystem:\n    rootdirectory: {}\n",
                root.display().to_string().replace('\\', "/")
            ),
        )
        .expect("config.yml");
    };
    write_config(&volume);
    let run = |flags: &[&str]| {
        std::process::Command::new(env!("CARGO_BIN_EXE_hologram"))
            .args(["oci", "garbage-collect"])
            .args(flags)
            .arg("--registry-config")
            .arg(&config)
            .env("HOME", dir.path())
            .env("USERPROFILE", dir.path())
            .env("HOLOGRAM_CONFIG_DIR", dir.path().join("config"))
            .output()
            .expect("hologram oci garbage-collect")
    };
    let dry = run(&["--dry-run"]);
    assert!(
        dry.status.success(),
        "{}",
        String::from_utf8_lossy(&dry.stderr)
    );
    let text = String::from_utf8_lossy(&dry.stdout).into_owned();
    assert!(text.lines().any(|line| line == "a/b"), "{text}");
    assert!(
        text.contains(&format!("blob eligible for deletion: {orphan}")),
        "{text}"
    );
    assert!(
        open(&volume).blob_stat(&repo("a/b"), &orphan).is_ok(),
        "a dry run removes nothing"
    );

    let quiet = run(&["--quiet"]);
    assert!(
        quiet.status.success(),
        "{}",
        String::from_utf8_lossy(&quiet.stderr)
    );
    assert!(quiet.stdout.is_empty(), "--quiet prints nothing");
    let store = open(&volume);
    assert!(store.blob_stat(&repo("a/b"), &orphan).is_err(), "swept");
    every_tag_pulls(&store, "a/b");

    // Beside a running server: refused, exit 5, nothing touched.
    let locked = run(&[]);
    assert_eq!(
        locked.status.code(),
        Some(5),
        "{}",
        String::from_utf8_lossy(&locked.stderr)
    );
    drop(store);

    // A volume nothing was pushed to: the reference's empty answer, nothing created.
    let empty = dir.path().join("empty");
    std::fs::create_dir(&empty).expect("empty volume");
    write_config(&empty);
    let fresh = run(&["--dry-run"]);
    assert!(
        fresh.status.success(),
        "{}",
        String::from_utf8_lossy(&fresh.stderr)
    );
    assert!(String::from_utf8_lossy(&fresh.stdout)
        .contains("0 blobs marked, 0 blobs and 0 manifests eligible for deletion"));
    assert_eq!(
        std::fs::read_dir(&empty).expect("dir").count(),
        0,
        "nothing created"
    );
}
