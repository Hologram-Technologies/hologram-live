//! A clean stop on `docker stop`, as the reference registry stops.
//!
//! In a container the server is process 1, which the kernel gives no default
//! signal handlers: without one, SIGTERM is ignored and the runtime kills the
//! process after its grace period, cutting uploads in flight. Registry mode
//! turns SIGTERM and SIGINT into the server's own graceful shutdown. A second
//! signal ends the process at once, for a drain that does not finish.

use crate::app::AppState;

/// Signal handlers, registered before the server exists: first thing in
/// `serve`, so a `docker stop` during startup is held, not lost, until
/// [`StopSignal::forward_to`] hands it on.
pub struct StopSignal {
    #[cfg(unix)]
    handlers: Option<(tokio::signal::unix::Signal, tokio::signal::unix::Signal)>,
}

impl StopSignal {
    /// Register SIGTERM and SIGINT now.
    #[must_use]
    pub fn register() -> Self {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{signal, SignalKind};
            let handlers = signal(SignalKind::terminate())
                .and_then(|terminate| Ok((terminate, signal(SignalKind::interrupt())?)))
                .map_err(|error| {
                    tracing::warn!(error = %error, "cannot watch SIGTERM; only Ctrl-C stops cleanly");
                })
                .ok();
            Self { handlers }
        }
        #[cfg(not(unix))]
        {
            Self {}
        }
    }

    /// On the first signal ask `state` to drain and stop; on the second, exit
    /// at once. `AppState::wait_shutdown` keeps a request made before the
    /// listeners wait, so a signal during startup still stops them.
    pub fn forward_to(mut self, state: AppState) {
        tokio::spawn(async move {
            self.next().await;
            tracing::info!("stop signal received; draining");
            state.request_shutdown();
            self.next().await;
            tracing::warn!("second stop signal; exiting without draining");
            std::process::exit(130);
        });
    }

    async fn next(&mut self) {
        #[cfg(unix)]
        if let Some((terminate, interrupt)) = self.handlers.as_mut() {
            tokio::select! {
                _ = terminate.recv() => {}
                _ = interrupt.recv() => {}
            }
            return;
        }
        let _ = tokio::signal::ctrl_c().await;
    }
}
