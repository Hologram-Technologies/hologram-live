//! A clean stop on `docker stop`, as the reference registry stops.
//!
//! In a container the server is process 1, which the kernel gives no default
//! signal handlers: without one, SIGTERM is ignored and the runtime kills the
//! process after its grace period, cutting uploads in flight. Registry mode
//! turns SIGTERM and SIGINT into the server's own graceful shutdown.

use crate::app::AppState;

/// Spawn the task that waits for SIGTERM or SIGINT (Ctrl-C) and asks the
/// server to drain and stop.
pub fn stop_on_signal(state: AppState) {
    tokio::spawn(async move {
        wait_for_signal().await;
        tracing::info!("stop signal received; draining");
        state.request_shutdown();
    });
}

#[cfg(unix)]
async fn wait_for_signal() {
    use tokio::signal::unix::{signal, SignalKind};
    match signal(SignalKind::terminate()) {
        Ok(mut terminate) => {
            tokio::select! {
                _ = terminate.recv() => {}
                _ = tokio::signal::ctrl_c() => {}
            }
        }
        Err(error) => {
            tracing::warn!(error = %error, "cannot watch SIGTERM; only Ctrl-C stops cleanly");
            let _ = tokio::signal::ctrl_c().await;
        }
    }
}

#[cfg(not(unix))]
async fn wait_for_signal() {
    let _ = tokio::signal::ctrl_c().await;
}
