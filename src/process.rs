use crate::config::AppConfig;
use crate::error::{LiveError, Result};
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
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
            .write(true)
            .create_new(true)
            .open(&lock_path)
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::AlreadyExists {
                    LiveError::Conflict(format!(
                        "another hologram daemon owns {}; remove it only after confirming no daemon is running",
                        lock_path.display()
                    ))
                } else {
                    LiveError::io(&lock_path, error)
                }
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
    let executable = std::env::current_exe()
        .map_err(|error| LiveError::Io(format!("resolve current executable: {error}")))?;
    let child = Command::new(executable)
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
    wait_ready(config).await?;
    Ok(child.id())
}

pub async fn ensure_daemon(config: &AppConfig, config_path: &Path) -> Result<()> {
    if !is_ready(config).await {
        start_daemon(config, config_path).await?;
    }
    Ok(())
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

async fn wait_ready(config: &AppConfig) -> Result<()> {
    for _ in 0..100 {
        if is_ready(config).await {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Err(LiveError::Transport(format!(
        "daemon did not become ready at {}",
        config.server.listen
    )))
}

async fn is_ready(config: &AppConfig) -> bool {
    tokio::time::timeout(
        Duration::from_millis(150),
        tokio::net::TcpStream::connect(&config.server.listen),
    )
    .await
    .is_ok_and(|result| result.is_ok())
}
