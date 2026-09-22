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

/// A layer from the shared pool, pushed by sha256 or, half the time, by
/// blake3 (linked under that name); a manifest names it by sha256 either way.
fn pool_layer(store: &OciStore, name: &str, rng: &mut Rng) -> Digest {
    let bytes = format!("layer {}", rng.below(6)).into_bytes();
    if rng.below(2) == 0 {
        return blob(store, name, &bytes);
    }
    let id = store.upload_begin(&repo(name)).expect("begin");
    store.upload_append(&id, 0, &bytes).expect("append");
    store
        .upload_finish(&id, &Digest::from_blake3(&blake3::hash(&bytes)))
        .expect("finish");
    Digest::sha256_of(&bytes)
}

/// Every signature attached to a tagged image is still listed and pulls.
fn every_signature_stays(store: &OciStore, name: &str, signed: &[(String, Digest, Digest)]) {
    for (repo_name, subject, signature) in
        signed.iter().filter(|(repo_name, _, _)| repo_name == name)
    {
        let still_tagged = store
            .tags_page(&repo(repo_name), None, 1000)
            .unwrap_or_default()
            .iter()
            .any(|tag| {
                store
                    .manifest_get(&repo(repo_name), &Reference::Tag(tag.clone()))
                    .is_ok_and(|manifest| manifest.digest == *subject)
            });
        if !still_tagged {
            continue;
        }
        let listed = store
            .referrers_of(&repo(repo_name), subject)
            .expect("referrers");
        assert!(
            listed
                .iter()
                .any(|referrer| referrer.digest == signature.as_str()),
            "{repo_name}: signature of {subject} lost"
        );
        assert!(store
            .manifest_get(&repo(repo_name), &Reference::Digest(signature.clone()))
            .is_ok());
    }
}

fn one_case(seed: u64) {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = open(dir.path());
    let mut rng = Rng(seed | 1);
    let repos = ["p/one", "p/two", "p/three"];
    let mut signed: Vec<(String, Digest, Digest)> = Vec::new();
    for step in 0..24 {
        let name = repos[usize::try_from(rng.below(3)).expect("index")];
        match rng.below(9) {
            // Push an image of one to three layers from a small shared pool, tagged.
            0..=2 => {
                let count = 1 + rng.below(3);
                let layers: Vec<Digest> = (0..count)
                    .map(|_| pool_layer(&store, name, &mut rng))
                    .collect();
                let refs: Vec<&Digest> = layers.iter().collect();
                let tag = format!("t{}", rng.below(4));
                manifest(&store, name, Some(&tag), &refs, None);
            }
            // Push untagged.
            3 => {
                let layer = pool_layer(&store, name, &mut rng);
                manifest(&store, name, None, &[&layer], None);
            }
            // Sign a tagged image: an untagged manifest whose subject it is.
            8 => {
                if let Some(tag) = store
                    .tags_page(&repo(name), None, 10)
                    .ok()
                    .and_then(|tags| tags.into_iter().next())
                {
                    let subject = store
                        .manifest_get(&repo(name), &Reference::Tag(tag))
                        .expect("tagged")
                        .digest;
                    let sig = blob(&store, name, format!("signature {step}").as_bytes());
                    let signature = manifest(&store, name, None, &[&sig], Some(&subject));
                    signed.push((name.to_owned(), subject, signature));
                }
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
                    every_signature_stays(&store, name, &signed);
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
                    every_signature_stays(&store, name, &signed);
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
        every_signature_stays(&store, name, &signed);
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

/// A layer pushed by blake3 (linked under that name) and named by sha256 in
/// a tagged manifest: its link must survive a collect (review of #106, 1).
#[test]
fn a_layer_named_by_its_other_digest_keeps_its_link() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = open(dir.path());
    let bytes = b"a layer pushed by blake3".to_vec();
    let id = store.upload_begin(&repo("a/b")).expect("begin");
    store.upload_append(&id, 0, &bytes).expect("append");
    let blake3 = Digest::from_blake3(&blake3::hash(&bytes));
    store.upload_finish(&id, &blake3).expect("finish");
    let sha256 = Digest::sha256_of(&bytes);
    manifest(&store, "a/b", Some("v1"), &[&sha256], None);
    collect(&store, GcOptions::default());
    collect(
        &store,
        GcOptions {
            dry_run: false,
            delete_untagged: true,
        },
    );
    every_tag_pulls(&store, "a/b");
    assert!(
        store.blob_stat(&repo("a/b"), &blake3).is_ok(),
        "the blake3 link is kept"
    );
}

/// An index that names its child by the child's other digest: the child's
/// layers are kept under --delete-untagged (review of #106, 2).
#[test]
fn an_index_child_named_by_its_other_digest_is_walked() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = open(dir.path());
    let layer = blob(&store, "i/app", b"the child's layer");
    let child = manifest(&store, "i/app", None, &[&layer], None);
    let child_blake3 = store
        .alias_of(&child)
        .expect("alias")
        .expect("a blake3 name");
    let index = format!(
        r#"{{"schemaVersion":2,"mediaType":"{INDEX}","manifests":[{{"mediaType":"{MANIFEST}","digest":"{child_blake3}","size":1}}]}}"#
    );
    let plan = ManifestPlan {
        kind: LinkKind::Index,
        must_exist: vec![child_blake3.clone()],
        subject: None,
    };
    store
        .manifest_put(
            &repo("i/app"),
            &Reference::parse("multi").expect("tag"),
            INDEX,
            index.as_bytes(),
            &plan,
        )
        .expect("index");
    collect(
        &store,
        GcOptions {
            dry_run: false,
            delete_untagged: true,
        },
    );
    assert!(store
        .manifest_get(&repo("i/app"), &Reference::Digest(child.clone()))
        .is_ok());
    assert!(
        store.blob_stat(&repo("i/app"), &layer).is_ok(),
        "the child's layer is kept"
    );
}

/// One image, untagged in one repository and tagged in another, with a
/// signature in the first: in the first it still pulls by digest and keeps
/// its signature (review of #106, 3).
#[test]
fn an_image_kept_by_another_repository_keeps_its_links_and_signatures_here() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = open(dir.path());
    // Enumerated first: untagged here.
    let layer_a = blob(&store, "a/first", b"shared image layer");
    let image = manifest(&store, "a/first", None, &[&layer_a], None);
    let sig_layer = blob(&store, "a/first", b"signature");
    let signature = manifest(&store, "a/first", None, &[&sig_layer], Some(&image));
    // Tagged in the second repository.
    blob(&store, "b/second", b"shared image layer");
    manifest(&store, "b/second", Some("v1"), &[&layer_a], None);

    collect(
        &store,
        GcOptions {
            dry_run: false,
            delete_untagged: true,
        },
    );
    let pulled = store
        .manifest_get(&repo("a/first"), &Reference::Digest(image.clone()))
        .expect("the image is kept in a/first");
    let value: serde_json::Value = serde_json::from_slice(&pulled.bytes).expect("json");
    let layer =
        Digest::parse(value["layers"][0]["digest"].as_str().expect("layer")).expect("digest");
    assert!(
        store.blob_stat(&repo("a/first"), &layer).is_ok(),
        "its layer link in a/first is kept"
    );
    let referrers = store
        .referrers_of(&repo("a/first"), &image)
        .expect("referrers");
    assert!(
        referrers
            .iter()
            .any(|referrer| referrer.digest == signature.as_str()),
        "its signature is kept"
    );
    assert!(store
        .manifest_get(&repo("a/first"), &Reference::Digest(signature))
        .is_ok());
    every_tag_pulls(&store, "b/second");
}

/// A swept blob pushed by blake3 loses the name it was recorded under and is
/// no longer served. The store's sha256 link to the same file stays: that
/// path may have held one of the store's own records, so the registry never
/// deletes by a second name (review of #106).
#[test]
fn a_swept_blake3_blob_is_removed_by_its_recorded_name() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = open(dir.path());
    let bytes = b"an orphan pushed by blake3".to_vec();
    let id = store.upload_begin(&repo("a/b")).expect("begin");
    store.upload_append(&id, 0, &bytes).expect("append");
    let blake3 = Digest::from_blake3(&blake3::hash(&bytes));
    store.upload_finish(&id, &blake3).expect("finish");
    let (algorithm, hex) = blake3.as_str().split_once(':').expect("digest");
    let path = dir
        .path()
        .join("kappa/blobs")
        .join(algorithm)
        .join(&hex[..2])
        .join(&hex[2..4])
        .join(hex);
    assert!(path.exists());
    collect(&store, GcOptions::default());
    assert!(!path.exists(), "the recorded name is gone");
    assert!(
        store.blob_stat(&repo("a/b"), &blake3).is_err(),
        "and nothing serves it"
    );
}

/// The store keeps records of its own beside the blobs. Pushing one's exact
/// bytes, by sha256 or by blake3, and leaving them in no manifest must never
/// get the record swept (errata E12; review of #106).
#[test]
fn the_stores_own_records_survive_a_push_of_their_bytes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = open(dir.path());
    let layer = blob(&store, "a/b", b"a layer with records beside it");
    manifest(&store, "a/b", Some("v1"), &[&layer], None);
    let blobs = dir.path().join("kappa/blobs");
    let records: Vec<(String, Vec<u8>)> = files(&blobs)
        .into_iter()
        .filter(|(_, content)| {
            content.as_slice() != b"a layer with records beside it"
                && !content.starts_with(b"{\"schemaVersion\"")
        })
        .collect();
    assert!(!records.is_empty(), "the store wrote records of its own");
    for (index, (_, content)) in records.iter().enumerate() {
        // Half by sha256, half by blake3; none put in a manifest.
        let id = store.upload_begin(&repo("x/pusher")).expect("begin");
        store.upload_append(&id, 0, content).expect("append");
        let claimed = if index % 2 == 0 {
            Digest::sha256_of(content)
        } else {
            Digest::from_blake3(&blake3::hash(content))
        };
        store.upload_finish(&id, &claimed).expect("finish");
    }
    collect(
        &store,
        GcOptions {
            dry_run: false,
            delete_untagged: true,
        },
    );
    let after = files(&blobs);
    for (path, content) in &records {
        assert_eq!(
            after.get(path),
            Some(content),
            "the store's record {path} survives"
        );
    }
    every_tag_pulls(&store, "a/b");
}
