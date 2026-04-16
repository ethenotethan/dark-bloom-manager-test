//! Signal handling for graceful shutdown

use tokio::sync::broadcast;
use tracing::info;

/// Handles Unix signals for the daemon
pub struct SignalHandler {
    shutdown_tx: broadcast::Sender<()>,
}

impl SignalHandler {
    /// Create a new signal handler
    pub fn new() -> (Self, broadcast::Receiver<()>) {
        let (shutdown_tx, shutdown_rx) = broadcast::channel(1);
        (Self { shutdown_tx }, shutdown_rx)
    }

    /// Start listening for signals
    pub async fn listen(&self) {
        use tokio::signal::unix::{signal, SignalKind};

        let mut sigterm =
            signal(SignalKind::terminate()).expect("Failed to register SIGTERM handler");
        let mut sigint =
            signal(SignalKind::interrupt()).expect("Failed to register SIGINT handler");
        let mut sighup = signal(SignalKind::hangup()).expect("Failed to register SIGHUP handler");

        tokio::select! {
            _ = sigterm.recv() => {
                info!("Received SIGTERM, initiating shutdown");
            }
            _ = sigint.recv() => {
                info!("Received SIGINT, initiating shutdown");
            }
            _ = sighup.recv() => {
                info!("Received SIGHUP, reloading configuration");
                // TODO: implement config reload
                return; // Don't shutdown on SIGHUP
            }
        }

        let _ = self.shutdown_tx.send(());
    }
}

impl Default for SignalHandler {
    fn default() -> Self {
        Self::new().0
    }
}
