//! Darkbloom CLI controller

use anyhow::{Context, Result};
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::process::Command;
use tracing::{debug, info, warn};

use super::{DarkbloomProcessStatus, EarningsInfo};
use crate::config::{DarkbloomConfig, ShutdownStrategy};

/// Controller for managing the Darkbloom provider process
pub struct Controller {
    binary_path: String,
    startup_timeout: Duration,
    shutdown_timeout: Duration,
    shutdown_strategy: ShutdownStrategy,
}

impl Controller {
    /// Create a new Darkbloom controller
    pub fn new(config: &DarkbloomConfig) -> Self {
        Self {
            binary_path: config.binary_path.clone(),
            startup_timeout: Duration::from_secs(config.startup_timeout_secs),
            shutdown_timeout: Duration::from_secs(config.shutdown_timeout_secs),
            shutdown_strategy: config.shutdown_strategy,
        }
    }

    /// Get the current status of Darkbloom
    pub async fn status(&self) -> Result<DarkbloomProcessStatus> {
        let output = Command::new(&self.binary_path)
            .arg("status")
            .arg("--json")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .context("Failed to run darkbloom status")?;

        if !output.status.success() {
            // Darkbloom not running or command failed
            debug!("darkbloom status failed: {:?}", output.status);
            return Ok(DarkbloomProcessStatus::default());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);

        // Try to parse JSON output
        match serde_json::from_str::<DarkbloomProcessStatus>(&stdout) {
            Ok(status) => Ok(status),
            Err(e) => {
                debug!("Failed to parse darkbloom status JSON: {}", e);
                // Try to determine if running from exit code alone
                Ok(DarkbloomProcessStatus {
                    running: output.status.success(),
                    ..Default::default()
                })
            }
        }
    }

    /// Check if Darkbloom is currently running
    pub async fn is_running(&self) -> bool {
        match self.status().await {
            Ok(status) => status.running,
            Err(_) => false,
        }
    }

    /// Check if Darkbloom is mid-inference (has an active request)
    pub async fn is_busy(&self) -> bool {
        match self.status().await {
            Ok(status) => status.active_request,
            Err(_) => false,
        }
    }

    /// Start Darkbloom provider
    pub async fn start(&self) -> Result<()> {
        info!("Starting Darkbloom provider");

        // Check if already running
        if self.is_running().await {
            debug!("Darkbloom is already running");
            return Ok(());
        }

        // Start the daemon
        let output = Command::new(&self.binary_path)
            .arg("start")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .context("Failed to run darkbloom start")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("darkbloom start failed: {}", stderr);
        }

        // Wait for healthy status
        let deadline = Instant::now() + self.startup_timeout;
        while Instant::now() < deadline {
            let status = self.status().await?;
            if status.running && status.connected {
                info!("Darkbloom provider started and connected");
                return Ok(());
            }
            if status.running {
                debug!("Darkbloom running but not yet connected, waiting...");
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }

        // Check final status
        let status = self.status().await?;
        if status.running {
            if status.connected {
                return Ok(());
            }
            warn!("Darkbloom started but not connected within timeout");
            return Ok(()); // Still consider it started
        }

        anyhow::bail!("Darkbloom failed to start within timeout");
    }

    /// Stop Darkbloom provider
    pub async fn stop(&self) -> Result<()> {
        info!("Stopping Darkbloom provider");

        // Check if already stopped
        if !self.is_running().await {
            debug!("Darkbloom is already stopped");
            return Ok(());
        }

        // Handle graceful shutdown if configured
        if self.shutdown_strategy == ShutdownStrategy::Graceful {
            // Wait for any active request to complete
            let deadline = Instant::now() + self.shutdown_timeout / 2;
            while Instant::now() < deadline {
                if !self.is_busy().await {
                    break;
                }
                debug!("Waiting for Darkbloom active request to complete...");
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }

        // Send stop command
        let output = Command::new(&self.binary_path)
            .arg("stop")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .context("Failed to run darkbloom stop")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            warn!("darkbloom stop returned error: {}", stderr);
            // Continue to verify it actually stopped
        }

        // Wait for process to actually stop
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            if !self.is_running().await {
                info!("Darkbloom provider stopped");
                return Ok(());
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }

        // Force kill if still running
        warn!("Darkbloom did not stop gracefully, attempting force stop");
        self.force_stop().await
    }

    /// Force stop Darkbloom (SIGKILL)
    async fn force_stop(&self) -> Result<()> {
        // Try to find and kill the process
        let output = Command::new("pkill")
            .args(["-9", "-f", "darkbloom"])
            .output()
            .await;

        match output {
            Ok(o) if o.status.success() => {
                info!("Force killed Darkbloom process");
                Ok(())
            }
            Ok(_) => {
                // pkill returns non-zero if no process found, which is fine
                Ok(())
            }
            Err(e) => {
                warn!("Failed to force kill Darkbloom: {}", e);
                Ok(()) // Don't fail the whole operation
            }
        }
    }

    /// Restart Darkbloom provider
    pub async fn restart(&self) -> Result<()> {
        self.stop().await?;
        tokio::time::sleep(Duration::from_secs(2)).await;
        self.start().await
    }

    /// Get earnings information
    pub async fn earnings(&self) -> Result<EarningsInfo> {
        let output = Command::new(&self.binary_path)
            .arg("earnings")
            .arg("--json")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .context("Failed to run darkbloom earnings")?;

        if !output.status.success() {
            debug!("darkbloom earnings failed: {:?}", output.status);
            return Ok(EarningsInfo::default());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);

        match serde_json::from_str::<EarningsInfo>(&stdout) {
            Ok(earnings) => Ok(earnings),
            Err(e) => {
                debug!("Failed to parse darkbloom earnings JSON: {}", e);
                Ok(EarningsInfo::default())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_controller_creation() {
        let config = DarkbloomConfig::default();
        let controller = Controller::new(&config);
        assert_eq!(controller.binary_path, "darkbloom");
    }
}
