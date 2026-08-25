use crate::config::AppConfig;
use crate::error::{LiveError, Result};
use fs4::{FileExt, TryLockError};
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::Duration;

pub struct DaemonGuard {
    lock_path: PathBuf,
    pid_path: PathBuf,
    _lock: File,
}

impl DaemonGuard {
    pub fn acquire(config: &AppConfig) -> Result<Self> {
        std::fs::create_dir_all(&config.paths.state_dir)
            .map_err(|error| LiveError::io(&config.paths.state_dir, error))?;
        let lock_path = config.paths.state_dir.join("hologram.lock");
        let pid_path = config.paths.state_dir.join("hologram.pid");
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|error| LiveError::io(&lock_path, error))?;
        FileExt::try_lock(&lock).map_err(|error| match error {
            TryLockError::WouldBlock => {
                let owner = read_pid(config)
                    .map(|pid| format!(" (pid {pid})"))
                    .unwrap_or_default();
                LiveError::Conflict(format!(
                    "another hologram daemon{owner} owns {}",
                    lock_path.display()
                ))
            }
            TryLockError::Error(error) => LiveError::io(&lock_path, error),
        })?;
        std::fs::write(&pid_path, std::process::id().to_string())
            .map_err(|error| LiveError::io(&pid_path, error))?;
        Ok(Self {
            lock_path,
            pid_path,
            _lock: lock,
        })
    }
}

impl Drop for DaemonGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.pid_path);
        // Remove the name while ownership is still held. Closing `_lock` then
        // releases the OS lock, including after an abnormal process exit.
        let _ = std::fs::remove_file(&self.lock_path);
    }
}

pub async fn start_daemon(config: &AppConfig, config_path: &Path) -> Result<u32> {
    if is_ready(config).await {
        return read_pid(config).or(Ok(0));
    }
    std::fs::create_dir_all(&config.paths.state_dir)
        .map_err(|error| LiveError::io(&config.paths.state_dir, error))?;
    let log_path = log_path(config);
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(|error| LiveError::io(&log_path, error))?;
    let log_start = log
        .metadata()
        .map_err(|error| LiveError::io(&log_path, error))?
        .len();
    let executable = std::env::current_exe()
        .map_err(|error| LiveError::Io(format!("resolve current executable: {error}")))?;
    let mut child = Command::new(executable)
        .arg("--config")
        .arg(config_path)
        .arg("serve")
        .stdin(Stdio::null())
        .stdout(Stdio::from(
            log.try_clone()
                .map_err(|error| LiveError::io(&log_path, error))?,
        ))
        .stderr(Stdio::from(log))
        .spawn()
        .map_err(|error| LiveError::Io(format!("start hologram daemon: {error}")))?;
    wait_ready(config, &mut child, &log_path, log_start).await?;
    Ok(child.id())
}

pub async fn ensure_daemon(config: &AppConfig, config_path: &Path) -> Result<()> {
    if !is_ready(config).await {
        start_daemon(config, config_path).await?;
    }
    Ok(())
}

/// Wait until the listener is closed and the daemon that owned it has
/// released its process-ownership files. The listener can close before the
/// server task finishes flushing telemetry and drops [`DaemonGuard`], so a
/// restart that only watches the port can race the retiring process on Linux.
pub async fn wait_stopped(config: &AppConfig, previous_pid: Option<u32>) -> Result<()> {
    for _ in 0..50 {
        let listener_closed = !is_ready(config).await;
        let owner_released =
            previous_pid.is_none_or(|pid| read_pid(config).ok().is_none_or(|owner| owner != pid));
        if listener_closed && owner_released {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let owner = previous_pid.map_or_else(String::new, |pid| format!(" pid {pid}"));
    Err(LiveError::Transport(format!(
        "daemon{owner} did not stop and release ownership within 5 seconds"
    )))
}

pub fn log_path(config: &AppConfig) -> PathBuf {
    config.paths.state_dir.join("hologram.log")
}

pub fn read_pid(config: &AppConfig) -> Result<u32> {
    let path = config.paths.state_dir.join("hologram.pid");
    let value = std::fs::read_to_string(&path).map_err(|error| LiveError::io(&path, error))?;
    value
        .trim()
        .parse()
        .map_err(|error| LiveError::Protocol(format!("parse daemon pid: {error}")))
}

async fn wait_ready(
    config: &AppConfig,
    child: &mut Child,
    log_path: &Path,
    log_start: u64,
) -> Result<()> {
    for _ in 0..100 {
        if is_ready(config).await {
            return Ok(());
        }
        if let Some(status) = child
            .try_wait()
            .map_err(|error| LiveError::Io(format!("inspect daemon process: {error}")))?
        {
            return Err(startup_error(config, status, log_path, log_start));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let _ = child.kill();
    let _ = child.wait();
    Err(LiveError::Transport(format!(
        "daemon did not become ready at {} within 5 seconds; {}",
        config.server.listen,
        log_diagnostics(log_path, log_start)
    )))
}

fn startup_error(
    config: &AppConfig,
    status: ExitStatus,
    log_path: &Path,
    log_start: u64,
) -> LiveError {
    LiveError::Transport(format!(
        "daemon exited with {status} before becoming ready at {}; {}",
        config.server.listen,
        log_diagnostics(log_path, log_start)
    ))
}

fn log_diagnostics(path: &Path, offset: u64) -> String {
    let Ok(bytes) = std::fs::read(path) else {
        return format!("daemon log: {}", path.display());
    };
    let start = usize::try_from(offset)
        .unwrap_or(usize::MAX)
        .min(bytes.len());
    let output = &bytes[start..];
    let tail = &output[output.len().saturating_sub(4_096)..];
    let message = String::from_utf8_lossy(tail);
    let message = message.trim();
    if message.is_empty() {
        format!("daemon log: {}", path.display())
    } else {
        format!("daemon log {}:\n{message}", path.display())
    }
}

async fn is_ready(config: &AppConfig) -> bool {
    tokio::time::timeout(
        Duration::from_millis(150),
        tokio::net::TcpStream::connect(&config.server.listen),
    )
    .await
    .is_ok_and(|result| result.is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::now_millis;

    #[test]
    fn daemon_guard_reclaims_stale_files_and_rejects_live_owners() {
        let root = std::env::temp_dir().join(format!(
            "hologram-daemon-guard-{}-{}",
            std::process::id(),
            now_millis()
        ));
        std::fs::create_dir_all(&root).expect("create test state directory");
        std::fs::write(root.join("hologram.lock"), b"stale").expect("write stale lock file");
        std::fs::write(root.join("hologram.pid"), b"999999").expect("write stale pid file");

        let mut config = AppConfig::default();
        config.paths.state_dir.clone_from(&root);

        let owner = DaemonGuard::acquire(&config).expect("reclaim stale ownership files");
        let error = DaemonGuard::acquire(&config)
            .err()
            .expect("reject a concurrently held daemon lock");
        assert!(matches!(error, LiveError::Conflict(_)));
        drop(owner);

        let replacement = DaemonGuard::acquire(&config).expect("acquire after owner exits");
        drop(replacement);
        std::fs::remove_dir_all(root).expect("remove test state directory");
    }

    #[test]
    fn startup_diagnostics_only_include_the_current_attempt() {
        let path = std::env::temp_dir().join(format!(
            "hologram-startup-log-{}-{}",
            std::process::id(),
            now_millis()
        ));
        std::fs::write(&path, b"old failure\ncurrent failure\n").expect("write test log");
        let diagnostics = log_diagnostics(&path, "old failure\n".len() as u64);
        assert!(!diagnostics.contains("old failure"));
        assert!(diagnostics.contains("current failure"));
        std::fs::remove_file(path).expect("remove test log");
    }

    #[tokio::test]
    async fn stopped_wait_includes_daemon_guard_release() {
        let root = std::env::temp_dir().join(format!(
            "hologram-daemon-stop-{}-{}",
            std::process::id(),
            now_millis()
        ));
        std::fs::create_dir_all(&root).expect("create test state directory");
        let pid_path = root.join("hologram.pid");
        std::fs::write(&pid_path, b"4242").expect("write pid");

        let mut config = AppConfig::default();
        config.paths.state_dir.clone_from(&root);
        config.server.listen = "127.0.0.1:0".to_owned();
        let remover = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            std::fs::remove_file(pid_path).expect("release ownership");
        });

        wait_stopped(&config, Some(4242))
            .await
            .expect("observe stopped daemon");
        remover.await.expect("join remover");
        std::fs::remove_dir_all(root).expect("remove test state directory");
    }
}
