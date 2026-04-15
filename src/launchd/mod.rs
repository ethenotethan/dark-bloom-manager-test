//! macOS launchd service management

use anyhow::{Context, Result};
use std::path::PathBuf;
use std::process::Command;

use crate::config::Config;

const LABEL: &str = "ai.darkbloom.manager";

fn plist_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Library/LaunchAgents")
        .join(format!("{}.plist", LABEL))
}

fn binary_path() -> Result<String> {
    std::env::current_exe()
        .context("Failed to get current executable path")
        .map(|p| p.to_string_lossy().to_string())
}

/// Install the launchd service
pub fn install(config: &Config) -> Result<()> {
    let plist_dir = plist_path().parent().unwrap().to_path_buf();
    std::fs::create_dir_all(&plist_dir)?;

    let binary = binary_path()?;
    let data_dir = config.data_dir()?;
    let log_out = data_dir.join("daemon.log");
    let log_err = data_dir.join("daemon.err");

    let plist_content = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{}</string>
        <string>run</string>
        <string>--foreground</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>{}</string>
    <key>StandardErrorPath</key>
    <string>{}</string>
    <key>ProcessType</key>
    <string>Background</string>
    <key>LowPriorityBackgroundIO</key>
    <true/>
</dict>
</plist>
"#,
        LABEL,
        binary,
        log_out.display(),
        log_err.display()
    );

    std::fs::write(plist_path(), plist_content)?;

    // Load the service
    Command::new("launchctl")
        .args(["load", "-w"])
        .arg(plist_path())
        .status()
        .context("Failed to load launchd service")?;

    Ok(())
}

/// Uninstall the launchd service
pub fn uninstall() -> Result<()> {
    // Stop if running
    let _ = stop();

    // Unload
    Command::new("launchctl")
        .args(["unload", "-w"])
        .arg(plist_path())
        .status()
        .ok();

    // Remove plist
    if plist_path().exists() {
        std::fs::remove_file(plist_path())?;
    }

    Ok(())
}

/// Start the launchd service
pub fn start() -> Result<()> {
    Command::new("launchctl")
        .args(["start", LABEL])
        .status()
        .context("Failed to start launchd service")?;
    Ok(())
}

/// Stop the launchd service
pub fn stop() -> Result<()> {
    Command::new("launchctl")
        .args(["stop", LABEL])
        .status()
        .context("Failed to stop launchd service")?;
    Ok(())
}

/// Check if the service is running
pub fn is_running() -> bool {
    Command::new("launchctl")
        .args(["list", LABEL])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
