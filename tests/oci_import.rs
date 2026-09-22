#![cfg(feature = "oci")]
//! `import` (plan P8 T2, FR-012): a Docker Registry volume, written file by
//! file to the reference's path spec (`registry/storage/paths.go`, v3.1.1),
//! copied in. Every tag pulls with its original digest; a second run adds
//! nothing; an interrupted run finishes; damaged bytes are named and skipped;
//! the source is never written.

use hologram_live::modules::oci::import_plan;
use hologram_live::oci_store::{
    Digest, ImportEvent, ImportReport, OciStore, OpenOptions, ProblemKind, Reference, RepoName,
};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

const OCI_MANIFEST: &str = "application/vnd.oci.image.manifest.v1+json";
const OCI_INDEX: &str = "application/vnd.oci.image.index.v1+json";
const DOCKER_MANIFEST: &str = "application/vnd.docker.distribution.manifest.v2+json";

/// A reference volume, built the way the reference lays it out.
struct Volume {
    root: PathBuf,
}

impl Volume {
    fn v2(&self) -> PathBuf {
        self.root.join("docker/registry/v2")
    }

    fn data(&self, digest: &Digest) -> PathBuf {
        let (algorithm, hex) = digest.as_str().split_once(':').expect("digest");
        self.v2()
            .join("blobs")
            .join(algorithm)
            .join(&hex[..2])
            .join(hex)
            .join("data")
    }

    fn put(&self, bytes: &[u8]) -> Digest {
        let digest = Digest::sha256_of(bytes);
        let data = self.data(&digest);
        std::fs::create_dir_all(data.parent().expect("parent")).expect("mkdir");
        std::fs::write(data, bytes).expect("write blob");
        digest
    }

    fn link(path: PathBuf, digest: &Digest) {
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        // The reference writes the digest with no newline.
        std::fs::write(path, digest.as_str()).expect("write link");
    }

    fn repo_dir(&self, repo: &str) -> PathBuf {
        self.v2().join("repositories").join(repo)
    }

    fn digest_path(dir: PathBuf, digest: &Digest) -> PathBuf {
        let (algorithm, hex) = digest.as_str().split_once(':').expect("digest");
        dir.join(algorithm).join(hex).join("link")
    }

    fn layer(&self, repo: &str, bytes: &[u8]) -> Digest {
        let digest = self.put(bytes);
        Self::link(
            Self::digest_path(self.repo_dir(repo).join("_layers"), &digest),
            &digest,
        );
        digest
    }

    fn manifest(&self, repo: &str, body: &str) -> Digest {
        let digest = self.put(body.as_bytes());
        Self::link(
            Self::digest_path(self.repo_dir(repo).join("_manifests/revisions"), &digest),
            &digest,
        );
        digest
    }

    fn tag(&self, repo: &str, tag: &str, digest: &Digest) {
        let dir = self.repo_dir(repo).join("_manifests/tags").join(tag);
        Self::link(dir.join("current/link"), digest);
        Self::link(Self::digest_path(dir.join("index"), digest), digest);
    }
}

fn image(
    media_type: Option<&str>,
    config: &Digest,
    layers: &[&Digest],
    subject: Option<&Digest>,
) -> String {
    let layer_type = if media_type == Some(DOCKER_MANIFEST) {
        "application/vnd.docker.image.rootfs.diff.tar.gzip"
    } else {
        "application/vnd.oci.image.layer.v1.tar+gzip"
    };
    let layers: Vec<String> = layers
        .iter()
        .map(|digest| format!(r#"{{"mediaType":"{layer_type}","digest":"{digest}","size":1}}"#))
        .collect();
    let media = media_type.map_or(String::new(), |media_type| {
        format!(r#""mediaType":"{media_type}","#)
    });
    let subject = subject.map_or(String::new(), |digest| {
        format!(r#","subject":{{"mediaType":"{OCI_MANIFEST}","digest":"{digest}","size":1}}"#)
    });
    format!(
        r#"{{"schemaVersion":2,{media}"config":{{"mediaType":"application/vnd.oci.image.config.v1+json","digest":"{config}","size":1}},"layers":[{}]{subject}}}"#,
        layers.join(",")
    )
}

/// What the corpus should look like once imported: every tag and its digest,
/// and every blob each repository links.
struct Expected {
    tags: Vec<(&'static str, &'static str, Digest)>,
    blobs: Vec<(&'static str, Digest, Vec<u8>)>,
    signature: (Digest, Digest),
    dangling: Digest,
}

/// A small corpus with what gate D names: a two-platform index, a signature
/// referrer, a Docker v2 manifest, an OCI manifest without `mediaType`, a
/// layer over one store frame, layers shared between repositories, nested
/// repository names, and a layer link the reference's own GC left behind.
fn corpus(volume: &Volume) -> Expected {
    let big: Vec<u8> = (0..9 * 1024 * 1024 + 17)
        .map(|index: usize| u8::try_from(index % 251).expect("byte"))
        .collect();
    let base = b"a base layer shared by every image".to_vec();
    let mut blobs = Vec::new();

    let app = "lib/app";
    let config_amd = volume.layer(app, b"{\"architecture\":\"amd64\"}");
    let config_arm = volume.layer(app, b"{\"architecture\":\"arm64\"}");
    let shared = volume.layer(app, &base);
    let big_digest = volume.layer(app, &big);
    let amd = volume.manifest(
        app,
        &image(
            Some(OCI_MANIFEST),
            &config_amd,
            &[&shared, &big_digest],
            None,
        ),
    );
    let arm = volume.manifest(
        app,
        &image(Some(OCI_MANIFEST), &config_arm, &[&shared], None),
    );
    let index = volume.manifest(
        app,
        &format!(
            r#"{{"schemaVersion":2,"mediaType":"{OCI_INDEX}","manifests":[{{"mediaType":"{OCI_MANIFEST}","digest":"{amd}","size":1,"platform":{{"architecture":"amd64","os":"linux"}}}},{{"mediaType":"{OCI_MANIFEST}","digest":"{arm}","size":1,"platform":{{"architecture":"arm64","os":"linux"}}}}]}}"#
        ),
    );
    volume.tag(app, "v1", &index);
    let sig_config = volume.layer(app, b"{}");
    let sig_layer = volume.layer(app, b"a cosign signature");
    let signature = volume.manifest(
        app,
        &image(Some(OCI_MANIFEST), &sig_config, &[&sig_layer], Some(&amd)),
    );
    // A layer link whose blob the reference's garbage collection removed.
    let dangling = Digest::sha256_of(b"collected long ago");
    Volume::link(
        Volume::digest_path(volume.repo_dir(app).join("_layers"), &dangling),
        &dangling,
    );
    for (digest, bytes) in [
        (&config_amd, b"{\"architecture\":\"amd64\"}".to_vec()),
        (&shared, base.clone()),
        (&big_digest, big),
        (&sig_layer, b"a cosign signature".to_vec()),
    ] {
        blobs.push((app, digest.clone(), bytes));
    }

    let sub = "lib/app/sub";
    let sub_config = volume.layer(sub, b"{\"os\":\"linux\"}");
    let sub_shared = volume.layer(sub, &base);
    let plain = volume.manifest(sub, &image(None, &sub_config, &[&sub_shared], None));
    volume.tag(sub, "latest", &plain);
    blobs.push((sub, sub_shared, base.clone()));

    let docker = "tools";
    let docker_config = volume.layer(docker, b"{\"docker\":true}");
    let docker_layer = volume.layer(docker, b"a docker layer");
    let docker_manifest = volume.manifest(
        docker,
        &image(
            Some(DOCKER_MANIFEST),
            &docker_config,
            &[&docker_layer],
            None,
        ),
    );
    volume.tag(docker, "1.0", &docker_manifest);
    volume.tag(docker, "stable", &docker_manifest);
    blobs.push((docker, docker_layer, b"a docker layer".to_vec()));

    Expected {
        tags: vec![
            (app, "v1", index),
            (sub, "latest", plain),
            (docker, "1.0", docker_manifest.clone()),
            (docker, "stable", docker_manifest),
        ],
        blobs,
        signature: (amd, signature),
        dangling,
    }
}

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

fn run(store: &OciStore, source: &Path) -> ImportReport {
    store
        .import(source, &import_plan, &mut |_| {})
        .expect("import")
}

fn repo(name: &str) -> RepoName {
    RepoName::parse(name).expect("repository name")
}

fn read_blob(store: &OciStore, name: &str, digest: &Digest) -> Vec<u8> {
    let (_, mut reader) = store.blob_open(&repo(name), digest).expect("blob");
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).expect("read");
    bytes
}

/// Every tag resolves to its original digest, every index child and blob
/// reads back byte for byte, and the signature is listed.
fn everything_pulls(store: &OciStore, expected: &Expected) {
    for (name, tag, digest) in &expected.tags {
        let manifest = store
            .manifest_get(&repo(name), &Reference::parse(tag).expect("tag"))
            .expect("tag pulls");
        assert_eq!(&manifest.digest, digest, "{name}:{tag}");
        assert_eq!(Digest::sha256_of(&manifest.bytes), *digest);
        let body: serde_json::Value = serde_json::from_slice(&manifest.bytes).expect("json");
        for child in body["manifests"].as_array().into_iter().flatten() {
            let child = Digest::parse(child["digest"].as_str().expect("digest")).expect("digest");
            store
                .manifest_get(&repo(name), &Reference::Digest(child))
                .expect("index child");
        }
    }
    for (name, digest, bytes) in &expected.blobs {
        assert_eq!(&read_blob(store, name, digest), bytes, "{name} {digest}");
    }
    let (subject, signature) = &expected.signature;
    let listed = store
        .referrers_of(&repo("lib/app"), subject)
        .expect("referrers");
    assert!(
        listed
            .iter()
            .any(|referrer| referrer.digest == signature.as_str()),
        "{listed:?}"
    );
}

#[test]
fn a_reference_volume_imports_and_every_tag_pulls_with_its_digest() {
    let dir = tempfile::tempdir().expect("tempdir");
    let volume = Volume {
        root: dir.path().join("reference"),
    };
    let expected = corpus(&volume);
    let store = open(&dir.path().join("ours"));
    let report = run(&store, &volume.root);
    assert!(!report.failed(), "{report:#?}");
    assert_eq!(report.repositories, 3);
    assert_eq!(report.manifests, 6);
    assert_eq!(report.tags, 4);
    // The shared base layer is copied once and linked in the second repository.
    assert_eq!(report.blobs_linked, 1, "{report:#?}");
    let missing: Vec<_> = report
        .problems
        .iter()
        .filter(|problem| problem.kind == ProblemKind::Missing)
        .collect();
    assert_eq!(missing.len(), 1, "{report:#?}");
    assert!(missing[0].detail.contains(expected.dangling.as_str()));
    everything_pulls(&store, &expected);
    let tags = store.tags_page(&repo("tools"), None, 10).expect("tags");
    assert_eq!(
        tags.iter()
            .map(hologram_live::oci_store::Tag::as_str)
            .collect::<Vec<_>>(),
        ["1.0", "stable"]
    );
}

#[test]
fn second_run_adds_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let volume = Volume {
        root: dir.path().join("reference"),
    };
    let expected = corpus(&volume);
    let store = open(&dir.path().join("ours"));
    run(&store, &volume.root);
    let again = run(&store, &volume.root);
    assert_eq!(again.added(), 0, "{again:#?}");
    assert_eq!(again.bytes_copied, 0);
    assert!(!again.failed());
    everything_pulls(&store, &expected);
}

/// Stopped part way (a panic mid-repository stands in for a kill: nothing
/// after it runs), the next run finishes with the same end state.
#[test]
fn interrupted_run_finishes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let volume = Volume {
        root: dir.path().join("reference"),
    };
    let expected = corpus(&volume);
    let ours = dir.path().join("ours");
    {
        let store = open(&ours);
        let stopped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            // The dangling link is reported while `lib/app`'s layers are
            // being copied: before any manifest.
            store.import(&volume.root, &import_plan, &mut |event| {
                if matches!(event, ImportEvent::Problem(_)) {
                    panic!("stop here");
                }
            })
        }));
        assert!(stopped.is_err());
        assert!(store
            .manifest_get(&repo("lib/app"), &Reference::parse("v1").expect("tag"))
            .is_err());
    }
    let store = open(&ours);
    let report = run(&store, &volume.root);
    assert!(!report.failed(), "{report:#?}");
    assert!(
        report.blobs_copied > 0 && report.manifests == 6,
        "{report:#?}"
    );
    everything_pulls(&store, &expected);
}

#[test]
fn a_damaged_source_blob_is_named_and_skipped() {
    let dir = tempfile::tempdir().expect("tempdir");
    let volume = Volume {
        root: dir.path().join("reference"),
    };
    let expected = corpus(&volume);
    // Flip a byte of the Docker image's layer.
    let (_, damaged, _) = expected
        .blobs
        .iter()
        .find(|(name, _, _)| *name == "tools")
        .expect("tools layer");
    let data = volume.data(damaged);
    let mut bytes = std::fs::read(&data).expect("read");
    bytes[0] ^= 1;
    std::fs::write(&data, bytes).expect("write");

    let store = open(&dir.path().join("ours"));
    let report = run(&store, &volume.root);
    assert!(report.failed());
    let mismatch: Vec<_> = report
        .problems
        .iter()
        .filter(|problem| problem.kind == ProblemKind::Mismatch)
        .collect();
    assert_eq!(mismatch.len(), 1, "{report:#?}");
    assert_eq!(mismatch[0].path, data);
    assert!(
        store.blob_stat(&repo("tools"), damaged).is_err(),
        "the damaged blob is not linked"
    );
    // Its manifest cannot come over, and its tags are named.
    assert!(report
        .problems
        .iter()
        .any(|problem| problem.kind == ProblemKind::Refused && problem.repository == "tools"));
    assert_eq!(
        report
            .problems
            .iter()
            .filter(|problem| problem.kind == ProblemKind::TagDangling)
            .count(),
        2
    );
    // Everything else did.
    for (name, tag, digest) in expected.tags.iter().filter(|(name, _, _)| *name != "tools") {
        let manifest = store
            .manifest_get(&repo(name), &Reference::parse(tag).expect("tag"))
            .expect("pulls");
        assert_eq!(&manifest.digest, digest);
    }
}

/// Every file of the source, with its bytes.
fn tree(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).expect("read_dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                pending.push(path);
            } else {
                files.insert(path.clone(), std::fs::read(&path).expect("read"));
            }
        }
    }
    files
}

#[test]
fn the_source_is_never_written() {
    let dir = tempfile::tempdir().expect("tempdir");
    let volume = Volume {
        root: dir.path().join("reference"),
    };
    corpus(&volume);
    let before = tree(&volume.root);
    let store = open(&dir.path().join("ours"));
    run(&store, &volume.root);
    assert_eq!(tree(&volume.root), before);
}

#[test]
fn a_directory_that_is_not_a_reference_volume_is_refused() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = open(&dir.path().join("ours"));
    let error = store
        .import(&dir.path().join("nothing"), &import_plan, &mut |_| {})
        .expect_err("refused");
    assert!(
        error.to_string().contains("not a Docker Registry volume"),
        "{error}"
    );
}

/// The command: the report, exit 1 when something did not come over, and
/// refusals for a source used as its own target and for a foreign directory.
#[test]
fn the_command() {
    let dir = tempfile::tempdir().expect("tempdir");
    let volume = Volume {
        root: dir.path().join("reference"),
    };
    let expected = corpus(&volume);
    let command = |source: &Path, into: &Path| {
        std::process::Command::new(env!("CARGO_BIN_EXE_hologram"))
            .args(["oci", "import"])
            .arg(source)
            .arg("--into")
            .arg(into)
            .env("HOME", dir.path())
            .env("USERPROFILE", dir.path())
            .env("HOLOGRAM_CONFIG_DIR", dir.path().join("config"))
            .output()
            .expect("hologram oci import")
    };
    let ours = dir.path().join("ours");
    let first = command(&volume.root, &ours);
    let text = String::from_utf8_lossy(&first.stdout).into_owned();
    assert!(
        first.status.success(),
        "{text}{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert!(text.starts_with("3 repositories: "), "{text}");
    assert!(text.contains(expected.dangling.as_str()), "{text}");
    let again = command(&volume.root, &ours);
    assert!(again.status.success());
    assert!(
        String::from_utf8_lossy(&again.stdout).contains(
            "0 blobs copied (0 bytes), 0 linked from another repository, 0 manifests, 0 tags"
        ),
        "{}",
        String::from_utf8_lossy(&again.stdout)
    );
    everything_pulls(&open(&ours), &expected);

    let (_, damaged, _) = expected
        .blobs
        .iter()
        .find(|(name, _, _)| *name == "tools")
        .expect("tools layer");
    std::fs::write(volume.data(damaged), b"not those bytes").expect("damage");
    let damaged_run = command(&volume.root, &dir.path().join("second"));
    assert_eq!(
        damaged_run.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&damaged_run.stderr)
    );
    let text = String::from_utf8_lossy(&damaged_run.stdout).into_owned();
    assert!(text.contains("mismatch tools"), "{text}");

    assert_eq!(
        command(&volume.root, &volume.root).status.code(),
        Some(2),
        "source as target"
    );
    let foreign = dir.path().join("foreign");
    std::fs::create_dir_all(&foreign).expect("mkdir");
    std::fs::write(foreign.join("notes.txt"), b"mine").expect("write");
    assert_eq!(
        command(&volume.root, &foreign).status.code(),
        Some(2),
        "a directory of something else"
    );
}
