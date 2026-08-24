use crate::config::AppConfig;
use crate::error::{LiveError, Result};
use crate::util::hex;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateArtifact {
    pub url: String,
    pub blake3: String,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateManifest {
    pub schema_version: u32,
    pub version: String,
    pub channel: String,
    pub protocol_min: u16,
    pub protocol_max: u16,
    pub artifacts: BTreeMap<String, UpdateArtifact>,
}

pub async fn check(config: &AppConfig) -> Result<UpdateManifest> {
    let url = config
        .update
        .manifest_url
        .as_deref()
        .ok_or_else(|| LiveError::Config("update.manifest_url is not configured".to_owned()))?;
    require_secure_url(url)?;
    let manifest = reqwest::Client::new()
        .get(url)
        .send()
        .await
        .map_err(|error| LiveError::Transport(format!("fetch update manifest: {error}")))?
        .error_for_status()
        .map_err(|error| LiveError::Transport(format!("fetch update manifest: {error}")))?
        .json::<UpdateManifest>()
        .await
        .map_err(|error| LiveError::Protocol(format!("decode update manifest: {error}")))?;
    if manifest.schema_version != 1 {
        return Err(LiveError::Protocol(format!(
            "unsupported update manifest schema {}",
            manifest.schema_version
        )));
    }
    if manifest.channel != config.update.channel {
        return Err(LiveError::Protocol(format!(
            "update manifest channel {:?} does not match configured channel {:?}",
            manifest.channel, config.update.channel
        )));
    }
    if !(manifest.protocol_min..=manifest.protocol_max).contains(&crate::protocol::PROTOCOL_VERSION)
    {
        return Err(LiveError::Protocol(
            "update is incompatible with this native protocol".to_owned(),
        ));
    }
    Ok(manifest)
}

pub async fn install(config: &AppConfig) -> Result<String> {
    let manifest = check(config).await?;
    let target = target_key();
    let artifact = manifest.artifacts.get(target).ok_or_else(|| {
        LiveError::NotFound(format!(
            "release {} has no artifact for {target}",
            manifest.version
        ))
    })?;
    require_secure_url(&artifact.url)?;
    let bytes = reqwest::Client::new()
        .get(&artifact.url)
        .send()
        .await
        .map_err(|error| LiveError::Transport(format!("download update: {error}")))?
        .error_for_status()
        .map_err(|error| LiveError::Transport(format!("download update: {error}")))?
        .bytes()
        .await
        .map_err(|error| LiveError::Transport(format!("read update: {error}")))?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) != artifact.size {
        return Err(LiveError::Protocol(
            "downloaded update size does not match manifest".to_owned(),
        ));
    }
    let digest = format!("blake3:{}", hex(blake3::hash(&bytes).as_bytes()));
    if digest != artifact.blake3 {
        return Err(LiveError::Protocol(
            "downloaded update digest does not match manifest".to_owned(),
        ));
    }
    replace_current_binary(&bytes)?;
    Ok(manifest.version)
}

pub fn rollback() -> Result<()> {
    let current = std::env::current_exe()
        .map_err(|error| LiveError::Io(format!("resolve current executable: {error}")))?;
    let previous = previous_path(&current);
    if !previous.exists() {
        return Err(LiveError::NotFound(format!(
            "no previous binary at {}",
            previous.display()
        )));
    }
    replace_from_path(&previous, &current)
}

fn replace_current_binary(bytes: &[u8]) -> Result<()> {
    #[cfg(windows)]
    {
        let _ = bytes;
        return Err(LiveError::Capability(
            "self-update replacement is not available on Windows; use install.ps1".to_owned(),
        ));
    }

    #[cfg(not(windows))]
    {
        use std::os::unix::fs::PermissionsExt;
        let current = std::env::current_exe()
            .map_err(|error| LiveError::Io(format!("resolve current executable: {error}")))?;
        let temporary = current.with_extension(format!("update.{}", std::process::id()));
        std::fs::write(&temporary, bytes).map_err(|error| LiveError::io(&temporary, error))?;
        std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o755))
            .map_err(|error| LiveError::io(&temporary, error))?;
        let previous = previous_path(&current);
        if previous.exists() {
            std::fs::remove_file(&previous).map_err(|error| LiveError::io(&previous, error))?;
        }
        std::fs::rename(&current, &previous).map_err(|error| LiveError::io(&current, error))?;
        if let Err(error) = std::fs::rename(&temporary, &current) {
            let _ = std::fs::rename(&previous, &current);
            return Err(LiveError::io(&current, error));
        }
        Ok(())
    }
}

fn replace_from_path(source: &Path, destination: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        let _ = (source, destination);
        Err(LiveError::Capability(
            "self-update rollback is not available on Windows; use install.ps1".to_owned(),
        ))
    }

    #[cfg(not(windows))]
    {
        let failed = destination.with_extension("failed");
        if failed.exists() {
            std::fs::remove_file(&failed).map_err(|error| LiveError::io(&failed, error))?;
        }
        std::fs::rename(destination, &failed).map_err(|error| LiveError::io(destination, error))?;
        if let Err(error) = std::fs::rename(source, destination) {
            let _ = std::fs::rename(&failed, destination);
            return Err(LiveError::io(destination, error));
        }
        Ok(())
    }
}

fn previous_path(current: &Path) -> PathBuf {
    current.with_extension("previous")
}

fn require_secure_url(url: &str) -> Result<()> {
    if url.starts_with("https://")
        || url.starts_with("http://127.0.0.1")
        || url.starts_with("http://localhost")
    {
        Ok(())
    } else {
        Err(LiveError::Config(format!(
            "update URL must use HTTPS unless it is loopback: {url}"
        )))
    }
}

const fn target_key() -> &'static str {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        "aarch64-apple-darwin"
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        "x86_64-apple-darwin"
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        "aarch64-unknown-linux-gnu"
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        "x86_64-unknown-linux-gnu"
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        "x86_64-pc-windows-msvc"
    }
    #[cfg(not(any(
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64"),
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "windows", target_arch = "x86_64")
    )))]
    {
        "unsupported"
    }
}
