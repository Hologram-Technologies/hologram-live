//! The administration listener of registry mode (ADR 028, FR-S06).
//!
//! The module API and gRPC, `shutdown` included, are served on a Unix socket
//! under the state directory, mode 0600, never on the public port. `docker
//! exec` reaches it; the network cannot. On Windows the named pipe is not
//! built yet: registry mode there serves no administration at all, and says so.

use crate::app::AppState;
#[cfg(unix)]
use crate::error::LiveError;
use crate::error::Result;
use axum::Router;
use std::path::Path;

#[cfg(unix)]
pub type Listener = tokio::net::UnixListener;

/// No administration listener: the named pipe is not built yet.
#[cfg(not(unix))]
pub struct Listener;

/// Bind the socket: its directory is made owner-only first, a socket left by a
/// stopped server is replaced, and anything else at the path stops the start.
///
/// # Errors
///
/// `LiveError::Transport` naming the path when it cannot be bound.
#[cfg(unix)]
pub fn bind(path: &Path) -> Result<Listener> {
    use std::os::unix::fs::{FileTypeExt, PermissionsExt};
    let fail = |what: &str, error: std::io::Error| {
        LiveError::Transport(format!(
            "administration socket {}: {what}: {error}",
            path.display()
        ))
    };
    if let Some(directory) = path.parent() {
        std::fs::create_dir_all(directory).map_err(|error| fail("create directory", error))?;
        std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))
            .map_err(|error| fail("make the directory owner-only", error))?;
    }
    match std::fs::symlink_metadata(path) {
        // The process lock is already held, so no live server owns it.
        Ok(metadata) if metadata.file_type().is_socket() => {
            std::fs::remove_file(path).map_err(|error| fail("remove the old socket", error))?;
        }
        Ok(_) => {
            return Err(LiveError::Transport(format!(
                "administration socket {}: something that is not a socket is at the path",
                path.display()
            )));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(fail("inspect", error)),
    }
    let listener = Listener::bind(path).map_err(|error| fail("bind", error))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(|error| fail("make the socket owner-only", error))?;
    tracing::info!(socket = %path.display(), "administration socket ready");
    Ok(listener)
}

/// On Windows: nothing to bind yet.
///
/// # Errors
///
/// None; the signature matches the Unix one.
#[cfg(not(unix))]
pub fn bind(path: &Path) -> Result<Listener> {
    tracing::warn!(
        socket = %path.display(),
        "administration over a named pipe is not built yet: this registry serves no administration"
    );
    Ok(Listener)
}

/// Serve administration until the shutdown signal.
///
/// # Errors
///
/// `LiveError::Transport` when serving fails.
#[cfg(unix)]
pub async fn serve(state: AppState, listener: Listener, router: Router) -> Result<()> {
    axum::serve(listener, router)
        .with_graceful_shutdown(async move { state.wait_shutdown().await })
        .await
        .map_err(|error| LiveError::Transport(format!("serve administration: {error}")))
}

/// On Windows: wait for the shutdown signal, serving nothing.
///
/// # Errors
///
/// None; the signature matches the Unix one.
#[cfg(not(unix))]
pub async fn serve(state: AppState, _listener: Listener, _router: Router) -> Result<()> {
    state.wait_shutdown().await;
    Ok(())
}

/// Remove the socket after the server stops, so nothing dials a dead path.
pub fn remove(path: &Path) {
    #[cfg(unix)]
    if let Err(error) = std::fs::remove_file(path) {
        if error.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!(socket = %path.display(), error = %error, "could not remove the administration socket");
        }
    }
    #[cfg(not(unix))]
    let _ = path;
}
