#![cfg(feature = "oci")]
//! Integration tests for the store adapter. Run on Linux, macOS and Windows.

use hologram_live::oci_store::{OciStore, OciStoreError, OpenOptions};
use std::time::Duration;

fn options() -> OpenOptions {
    OpenOptions {
        create: true,
        upload_max_age: Duration::from_hours(7 * 24),
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

// ---- Task 4 and 5: blobs in and out --------------------------------------

use hologram_live::oci_store::{AsyncBlob, Digest, RepoName, FRAME};
use std::io::{Read, Seek, SeekFrom};
use tokio::io::AsyncReadExt;

const MIB: usize = 1024 * 1024;

fn repo(name: &str) -> RepoName {
    RepoName::parse(name).expect("repository name")
}

/// Deterministic bytes that do not compress, produced a frame at a time.
fn frame(index: u64) -> Vec<u8> {
    let mut hasher = blake3::Hasher::new();
    hasher.update(&index.to_le_bytes());
    let mut out = vec![0_u8; FRAME];
    hasher.finalize_xof().fill(&mut out);
    out
}

/// Push `frames` frames into `name`; returns the sha256 and blake3 digests.
fn push_blob(store: &OciStore, name: &str, frames: u64) -> (Digest, Digest) {
    use sha2::Digest as _;
    let id = store.upload_begin(&repo(name)).expect("begin");
    let (mut sha, mut blake, mut offset) = (sha2::Sha256::new(), blake3::Hasher::new(), 0_u64);
    for index in 0..frames {
        let bytes = frame(index);
        sha.update(&bytes);
        blake.update(&bytes);
        offset = store.upload_append(&id, offset, &bytes).expect("append");
    }
    let mut hex = String::new();
    for byte in sha.finalize() {
        use std::fmt::Write as _;
        write!(hex, "{byte:02x}").expect("write to a string");
    }
    let sha256 = Digest::parse(&format!("sha256:{hex}")).expect("digest");
    assert_eq!(store.upload_finish(&id, &sha256).expect("finish"), sha256);
    (sha256, Digest::from_blake3(&blake.finalize()))
}

#[test]
fn a_blob_is_readable_only_through_a_repository_that_links_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = OciStore::open(dir.path(), options()).expect("open");
    let (digest, _) = push_blob(&store, "a/x", 1);
    assert!(matches!(
        store.blob_stat(&repo("b/y"), &digest),
        Err(OciStoreError::NotInRepository { .. })
    ));
    assert_eq!(
        store.blob_stat(&repo("a/x"), &digest).expect("stat").size,
        FRAME as u64
    );
}

#[test]
fn a_blob_pushed_by_sha256_opens_by_its_blake3_alias_and_back() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = OciStore::open(dir.path(), options()).expect("open");
    let (sha256, blake3) = push_blob(&store, "a/x", 1);
    let (stat, mut reader) = store
        .blob_open(&repo("a/x"), &blake3)
        .expect("open by blake3");
    assert_eq!(stat.stored_as, sha256);
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).expect("read");
    assert_eq!(bytes, frame(0));
    assert!(
        matches!(
            store.blob_stat(&repo("b/y"), &blake3),
            Err(OciStoreError::NotInRepository { .. })
        ),
        "an alias does not widen the scope"
    );
}

/// Counts what is read, to prove a range does not read the rest.
struct Counting<R> {
    inner: R,
    read: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

impl<R: Read> Read for Counting<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let count = self.inner.read(buffer)?;
        self.read
            .fetch_add(count as u64, std::sync::atomic::Ordering::Relaxed);
        Ok(count)
    }
}

impl<R: Seek> Seek for Counting<R> {
    fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(position)
    }
}

#[tokio::test]
async fn a_range_is_served_without_reading_the_rest() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = OciStore::open(dir.path(), options()).expect("open");
    let (digest, _) = push_blob(&store, "a/x", 16); // 64 MiB
    let (_, reader) = store.blob_open(&repo("a/x"), &digest).expect("open");
    let read = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let counting = Counting {
        inner: reader,
        read: read.clone(),
    };

    let mut body = AsyncBlob::new(Box::new(counting), 10 * MIB as u64, MIB as u64);
    let mut got = Vec::new();
    body.read_to_end(&mut got).await.expect("read");

    // 10 MiB into the stream is 2 MiB into frame 2.
    assert_eq!(got, &frame(2)[2 * MIB..3 * MIB]);
    assert!(read.load(std::sync::atomic::Ordering::Relaxed) <= 2 * MIB as u64);
}

/// Count the files of exactly `size` bytes under `dir`.
fn count(dir: &std::path::Path, size: u64, found: &mut usize) {
    for entry in std::fs::read_dir(dir).expect("read_dir").flatten() {
        let path = entry.path();
        if path.is_dir() {
            count(&path, size, found);
        } else if entry.metadata().is_ok_and(|meta| meta.len() == size) {
            *found += 1;
        }
    }
}

#[test]
fn the_same_blob_pushed_twice_at_once_ends_as_one_file_and_two_links() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = std::sync::Arc::new(OciStore::open(dir.path(), options()).expect("open"));
    let handles: Vec<_> = ["a/x", "b/y"]
        .into_iter()
        .map(|name| {
            let store = store.clone();
            std::thread::spawn(move || push_blob(&store, name, 2).0)
        })
        .collect();
    let digests: Vec<Digest> = handles
        .into_iter()
        .map(|h| h.join().expect("thread"))
        .collect();
    assert_eq!(digests[0], digests[1]);
    for name in ["a/x", "b/y"] {
        assert_eq!(
            store
                .blob_stat(&repo(name), &digests[0])
                .expect("stat")
                .size,
            2 * FRAME as u64
        );
    }
    // Exactly one file of that size under the blob root.
    let mut found = 0;
    count(&store.layout().blob_root(), 2 * FRAME as u64, &mut found);
    assert_eq!(found, 1, "deduplicated: one file, two links");
}

/// SC-007 in small: memory must not grow with the size of a layer.
#[cfg(target_os = "linux")]
#[test]
#[ignore = "2 GiB; run by the registry-os CI job"]
fn two_gib_stay_under_200_mb() {
    fn rss_bytes() -> u64 {
        let statm = std::fs::read_to_string("/proc/self/statm").expect("statm");
        let pages: u64 = statm
            .split_whitespace()
            .nth(1)
            .expect("rss")
            .parse()
            .expect("number");
        pages * 4096
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let store = OciStore::open(dir.path(), options()).expect("open");
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let peak = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let sampler = {
        let (stop, peak) = (stop.clone(), peak.clone());
        std::thread::spawn(move || {
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                peak.fetch_max(rss_bytes(), std::sync::atomic::Ordering::Relaxed);
                std::thread::sleep(Duration::from_millis(100));
            }
        })
    };
    push_blob(&store, "big/layer", 512);
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    sampler.join().expect("sampler");
    let peak_mb = peak.load(std::sync::atomic::Ordering::Relaxed) / (1024 * 1024);
    println!("SC-007 measurement: peak RSS {peak_mb} MB while pushing 2 GiB");
    assert!(peak_mb < 200, "peak RSS was {peak_mb} MB");
}

// ---- Task 6: sessions that survive a restart ------------------------------

use hologram_live::oci_store::{ResumeReport, UploadId};

/// The sha256 of the first `frames` known frames, as the child writes them.
fn sha_of_known_frames(frames: u64) -> Digest {
    use sha2::Digest as _;
    let mut sha = sha2::Sha256::new();
    for index in 0..frames {
        sha.update(frame(index));
    }
    let mut hex = String::new();
    for byte in sha.finalize() {
        use std::fmt::Write as _;
        write!(hex, "{byte:02x}").expect("write to a string");
    }
    Digest::parse(&format!("sha256:{hex}")).expect("digest")
}

#[test]
fn an_upload_resumes_after_the_process_is_killed() {
    use std::io::BufRead;
    let dir = tempfile::tempdir().expect("tempdir");
    // The child opens the store, begins an upload, appends 3 known frames,
    // prints the upload id and then sleeps until it is killed.
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_oci_child"))
        .arg(dir.path())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("spawn");
    let mut id = String::new();
    std::io::BufReader::new(child.stdout.as_mut().expect("stdout"))
        .read_line(&mut id)
        .expect("read the upload id");
    child.kill().expect("kill"); // SIGKILL on Unix, TerminateProcess on Windows
    child.wait().expect("wait");

    let store = OciStore::open(dir.path(), options()).expect("reopen");
    let id = UploadId::parse(id.trim()).expect("id");
    let status = store.upload_status(&id).expect("the session survived");
    assert_eq!(status.received, 3 * FRAME as u64);
    store
        .upload_append(&id, status.received, &frame(3))
        .expect("append the rest");
    let digest = sha_of_known_frames(4);
    assert_eq!(store.upload_finish(&id, &digest).expect("finish"), digest);
    // The running blake3 was lost with the process; the alias is found by one read.
    let (stat, _) = store
        .blob_open(&repo("a/x"), &digest)
        .expect("linked in its repository");
    assert_eq!(stat.size, 4 * FRAME as u64);
}

#[test]
fn a_staging_file_with_no_row_and_a_row_with_no_file_are_both_cleaned() {
    let dir = tempfile::tempdir().expect("tempdir");
    let orphan_row = {
        let store = OciStore::open(dir.path(), options()).expect("open");
        let id = store.upload_begin(&repo("a/x")).expect("begin");
        store.upload_append(&id, 0, b"abc").expect("append");
        id
    };
    let staging = dir.path().join("kappa/staging");
    std::fs::remove_file(staging.join(orphan_row.as_str())).expect("remove the staged file");
    std::fs::write(
        staging.join("6f1c2a9e-3b7d-4c55-9f0a-2d8e7b6a5c41"),
        b"stray",
    )
    .expect("plant");

    let store = OciStore::open(dir.path(), options()).expect("reopen");

    assert!(matches!(
        store.upload_status(&orphan_row),
        Err(OciStoreError::UnknownUpload(_))
    ));
    assert_eq!(std::fs::read_dir(&staging).expect("read_dir").count(), 0);
    // Opening again finds nothing left to do.
    drop(store);
    let store = OciStore::open(dir.path(), options()).expect("open a third time");
    assert_eq!(
        store.resume_uploads().expect("resume"),
        ResumeReport::default()
    );
}

#[test]
fn an_upload_untouched_for_longer_than_the_purge_age_is_aborted() {
    let dir = tempfile::tempdir().expect("tempdir");
    let short = OpenOptions {
        create: true,
        upload_max_age: Duration::from_secs(1),
    };
    let store = OciStore::open(dir.path(), short).expect("open");
    let id = store.upload_begin(&repo("a/x")).expect("begin");
    store.upload_append(&id, 0, b"abc").expect("append");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock");
    let now_ms = u64::try_from(now.as_millis()).expect("fits");

    assert_eq!(
        store.purge_expired_uploads(now_ms).expect("purge"),
        0,
        "still fresh"
    );
    assert_eq!(
        store.purge_expired_uploads(now_ms + 2_000).expect("purge"),
        1
    );
    assert!(matches!(
        store.upload_status(&id),
        Err(OciStoreError::UnknownUpload(_))
    ));
}
