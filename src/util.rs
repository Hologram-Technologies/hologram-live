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

pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| LiveError::io(parent, error))?;
    }
    let temporary = path.with_extension(format!("tmp.{}", std::process::id()));
    std::fs::write(&temporary, bytes).map_err(|error| LiveError::io(&temporary, error))?;
    if path.exists() {
        std::fs::remove_file(path).map_err(|error| LiveError::io(path, error))?;
    }
    std::fs::rename(&temporary, path).map_err(|error| LiveError::io(path, error))
}

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
