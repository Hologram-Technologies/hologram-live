use crate::error::{LiveError, Result};
use std::env;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn home_dir() -> PathBuf {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map_or_else(|| PathBuf::from("."), PathBuf::from)
}

pub fn expand_home(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    let value = path.to_string_lossy();
    if value == "~" {
        return home_dir();
    }
    if let Some(rest) = value
        .strip_prefix("~/")
        .or_else(|| value.strip_prefix("~\\"))
    {
        return home_dir().join(rest);
    }
    path.to_path_buf()
}

/// Replaces `path` with `bytes` without ever leaving it absent.
///
/// Two properties, both previously missing:
///
/// The destination is never unlinked. `std::fs::rename` replaces an existing
/// file on every platform this ships to, so the previous `remove_file` bought
/// nothing and opened a window in which a crash left no file at all — which for
/// `cluster-peers.json` meant a restarted node with no peers to dial.
///
/// Each call uses its own temporary name. Two threads writing one path used to
/// share `tmp.<pid>`, so one would rename the file out from under the other and
/// the loser failed with `ENOENT`.
///
/// This does **not** `fsync`, deliberately. Measured on this repository's
/// cluster suite, flushing every write made `the_joiner_comes_to_admit_the_seed`
/// fail two runs in three: `NodeDirectory` holds a mutex across its write and is
/// rewritten every heartbeat round per peer, so a per-write flush serialises
/// tens of milliseconds into each round and convergence misses its deadline.
/// Callers whose state must survive a crash — and which write rarely — use
/// [`atomic_write_durable`] instead.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;

    static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| LiveError::io(parent, error))?;
    }
    let temporary = path.with_extension(format!(
        "tmp.{}.{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));

    let written = std::fs::File::create(&temporary).and_then(|mut file| file.write_all(bytes));
    if let Err(error) = written {
        // Leave nothing behind beside real state.
        let _ = std::fs::remove_file(&temporary);
        return Err(LiveError::io(&temporary, error));
    }

    if let Err(error) = std::fs::rename(&temporary, path) {
        let _ = std::fs::remove_file(&temporary);
        return Err(LiveError::io(path, error));
    }
    Ok(())
}

/// Like [`atomic_write`], and additionally flushes the file and its directory so
/// the result survives power loss.
///
/// For state that is expensive or impossible to reconstruct and is written
/// rarely. Do not use it on a hot path: that is what made a blanket flush
/// unworkable, and the reasoning is recorded on [`atomic_write`].
pub fn atomic_write_durable(path: &Path, bytes: &[u8]) -> Result<()> {
    atomic_write(path, bytes)?;
    if let Ok(file) = std::fs::File::open(path) {
        // Best effort, like the directory flush below: the bytes are already
        // visible to every reader, so failing the write here would turn a
        // durability shortfall into an outage.
        let _ = file.sync_all();
    }
    sync_parent(path);
    Ok(())
}

/// Flushes the directory entry so a completed rename survives power loss.
///
/// Best effort by design: if this fails the file is already correct as far as
/// every reader is concerned, and failing the write would turn a durability
/// shortfall into an outage.
#[cfg(unix)]
fn sync_parent(path: &Path) {
    if let Some(parent) = path.parent() {
        if let Ok(directory) = std::fs::File::open(parent) {
            let _ = directory.sync_all();
        }
    }
}

#[cfg(not(unix))]
fn sync_parent(_path: &Path) {}

pub fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

pub fn hex(bytes: &[u8]) -> String {
    const TABLE: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(TABLE[(byte >> 4) as usize]));
        output.push(char::from(TABLE[(byte & 0x0f) as usize]));
    }
    output
}

/// Install the process-wide rustls crypto provider before any HTTPS client is
/// built.
///
/// reqwest 0.13 made a provider mandatory: its `rustls` feature bakes in
/// aws-lc-rs, and the `rustls-no-provider` feature this crate uses instead
/// *panics* when a client is constructed with no provider installed. This tree
/// standardizes on ring — see the `tls-ring` features on `tonic` and
/// `opentelemetry-otlp` — so the default build stays pure Rust with no C
/// toolchain, which rules aws-lc-rs out.
///
/// Every reqwest client construction site calls this first. Doing it here
/// rather than only in `main` keeps unit tests, which build clients directly,
/// working without each one repeating the setup.
///
/// Root certificates are a separate concern and are *not* configured here.
/// reqwest 0.13 removed its roots features in favour of
/// `rustls-platform-verifier`, so HTTPS trust now comes from the operating
/// system trust store rather than a bundled root set. That is a behavioural
/// change for verified update downloads and mediated Component fetch, and no
/// test covers it: asserting on real trust anchors would require network
/// access or a fabricated CA, either of which tests the fixture rather than
/// the product.
pub fn install_crypto_provider() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        // Errors only when a provider is already installed, which is exactly
        // the state this function exists to guarantee.
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

pub fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut difference = 0_u8;
    for (left, right) in left.iter().zip(right) {
        difference |= left ^ right;
    }
    difference == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_written_file_holds_exactly_the_bytes() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("state").join("value.json");
        atomic_write(&path, b"{\"a\":1}").expect("write");
        assert_eq!(std::fs::read(&path).expect("read"), b"{\"a\":1}");
    }

    #[test]
    fn a_rewrite_replaces_the_contents_and_leaves_no_temporary() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("value.json");
        atomic_write(&path, b"first").expect("first write");
        atomic_write(&path, b"second").expect("second write");
        assert_eq!(std::fs::read(&path).expect("read"), b"second");
        assert_eq!(
            temporaries_beside(&path),
            0,
            "a successful write must leave no temporary file behind"
        );
    }

    // The defect this guards: the previous implementation removed the
    // destination before renaming, so a process killed in that window left the
    // file absent entirely rather than holding its previous contents.
    #[cfg(unix)]
    #[test]
    fn a_failed_write_leaves_the_original_contents_intact() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().expect("temporary directory");
        let parent = directory.path().join("state");
        std::fs::create_dir_all(&parent).expect("create parent");
        let path = parent.join("value.json");
        atomic_write(&path, b"original").expect("seed the file");

        // Deny writes to the directory so the temporary file cannot be created.
        std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o500))
            .expect("make the directory read-only");
        let failure = atomic_write(&path, b"replacement");
        std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o700))
            .expect("restore the directory");

        assert!(failure.is_err(), "the write should have failed");
        assert_eq!(
            std::fs::read(&path).expect("read"),
            b"original",
            "a failed write must not disturb the existing file"
        );
        assert_eq!(temporaries_beside(&path), 0, "a failed write must clean up");
    }

    // Two threads writing one path previously collided on a single temporary
    // name derived only from the process id.
    #[test]
    fn concurrent_writers_never_produce_a_torn_file() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("value.json");
        let payloads: Vec<Vec<u8>> = (0..8)
            .map(|index| vec![b'a' + u8::try_from(index).expect("small index"); 4096])
            .collect();

        std::thread::scope(|scope| {
            for payload in &payloads {
                let path = path.clone();
                scope.spawn(move || atomic_write(&path, payload).expect("concurrent write"));
            }
        });

        let written = std::fs::read(&path).expect("read");
        assert!(
            payloads.contains(&written),
            "the file must equal exactly one writer's payload, never a mixture"
        );
        assert_eq!(
            temporaries_beside(&path),
            0,
            "every writer must clean up its own temporary"
        );
    }

    #[test]
    fn the_durable_variant_writes_the_same_bytes() {
        // Durability itself is not observable from inside the process; this
        // pins the contract that the durable path is otherwise equivalent.
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("state").join("durable.json");
        atomic_write_durable(&path, b"durable").expect("durable write");
        assert_eq!(std::fs::read(&path).expect("read"), b"durable");
        assert_eq!(temporaries_beside(&path), 0);
    }

    fn temporaries_beside(path: &Path) -> usize {
        let parent = path.parent().expect("parent");
        std::fs::read_dir(parent)
            .expect("read directory")
            .filter_map(std::result::Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().contains(".tmp."))
            .count()
    }

    use super::install_crypto_provider;

    /// reqwest 0.13 under `rustls-no-provider` *panics* when a client is built
    /// with no crypto provider installed, and `cargo check` cannot see it: the
    /// failure only appears at runtime. Seven ollama tests were taken out by
    /// exactly this before `install_crypto_provider` existed.
    ///
    /// Both client forms are constructed here because they are separate code
    /// paths in reqwest, and this crate uses both — blocking for mediated
    /// Component fetch and the registry client, async for updates and the
    /// Ollama engine.
    #[test]
    fn a_tls_client_can_be_built_in_both_forms() {
        install_crypto_provider();

        reqwest::blocking::Client::builder()
            .https_only(true)
            .build()
            .expect("a blocking HTTPS client must build once a provider is installed");

        reqwest::Client::builder()
            .https_only(true)
            .build()
            .expect("an async HTTPS client must build once a provider is installed");
    }

    /// The installer runs from every client construction site rather than once
    /// in `main`, so it is called repeatedly and from many threads. It must be
    /// idempotent under that, not merely on the happy path.
    #[test]
    fn installing_the_provider_is_idempotent_across_threads() {
        let threads: Vec<_> = (0..8)
            .map(|_| std::thread::spawn(install_crypto_provider))
            .collect();
        for thread in threads {
            thread.join().expect("installing a provider must not panic");
        }

        // A client still builds afterwards, proving the repeated calls left a
        // working provider rather than a half-installed one.
        reqwest::blocking::Client::builder()
            .build()
            .expect("client builds after repeated installation");
    }
}
