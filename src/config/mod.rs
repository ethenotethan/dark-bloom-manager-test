//! Configuration management

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Main configuration structure
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub daemon: DaemonConfig,
    pub dashboard: DashboardConfig,
    pub omlx: OmlxConfig,
    pub darkbloom: DarkbloomConfig,
    pub memory: MemoryConfig,
    pub analytics: AnalyticsConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            daemon: DaemonConfig::default(),
            dashboard: DashboardConfig::default(),
            omlx: OmlxConfig::default(),
            darkbloom: DarkbloomConfig::default(),
            memory: MemoryConfig::default(),
            analytics: AnalyticsConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DaemonConfig {
    pub log_level: String,
    pub data_dir: PathBuf,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            log_level: "info".to_string(),
            data_dir: default_data_dir(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DashboardConfig {
    pub enabled: bool,
    pub port: u16,
    pub bind: String,
}

impl Default for DashboardConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            port: 9090,
            bind: "127.0.0.1".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OmlxConfig {
    pub endpoint: String,
    pub api_key: Option<String>,
    pub poll_interval_secs: u64,
    pub idle_threshold_secs: u64,
    pub min_idle_polls: u32,
    pub request_timeout_secs: u64,
    pub unreachable_behavior: UnreachableBehavior,
}

impl Default for OmlxConfig {
    fn default() -> Self {
        Self {
            endpoint: "http://localhost:8000".to_string(),
            api_key: None,
            poll_interval_secs: 5,
            idle_threshold_secs: 60,
            min_idle_polls: 3,
            request_timeout_secs: 5,
            unreachable_behavior: UnreachableBehavior::AssumeActive,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UnreachableBehavior {
    AssumeActive,
    AssumeIdle,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DarkbloomConfig {
    pub binary_path: String,
    pub model: String,
    pub startup_timeout_secs: u64,
    pub shutdown_timeout_secs: u64,
    pub shutdown_strategy: ShutdownStrategy,
    pub health_check_interval_secs: u64,
    pub model_ram_gb: f64,
}

impl Default for DarkbloomConfig {
    fn default() -> Self {
        Self {
            binary_path: "darkbloom".to_string(),
            model: "qwen3.5-27b-claude-opus-8bit".to_string(),
            startup_timeout_secs: 60,
            shutdown_timeout_secs: 120,
            shutdown_strategy: ShutdownStrategy::Graceful,
            health_check_interval_secs: 10,
            model_ram_gb: 36.0,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ShutdownStrategy {
    Graceful,
    Immediate,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MemoryConfig {
    pub min_available_gb: f64,
    pub check_interval_secs: u64,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            min_available_gb: 40.0,
            check_interval_secs: 2,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AnalyticsConfig {
    pub enabled: bool,
    pub snapshot_interval_secs: u64,
    pub retention_days: u32,
}

impl Default for AnalyticsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            snapshot_interval_secs: 60,
            retention_days: 90,
        }
    }
}

/// CLI overrides that can be applied to config
#[derive(Debug, Default, Clone)]
pub struct ConfigOverrides {
    // OMLX settings
    pub omlx_endpoint: Option<String>,
    pub omlx_port: Option<u16>,
    pub omlx_api_key: Option<String>,
    pub idle_threshold: Option<u64>,

    // Darkbloom settings
    pub darkbloom_binary: Option<String>,
    pub darkbloom_model: Option<String>,
    pub darkbloom_model_ram: Option<f64>,

    // Dashboard settings
    pub dashboard_port: Option<u16>,
    pub dashboard_disabled: bool,

    // Memory settings
    pub min_available_memory: Option<f64>,
}

impl Config {
    /// Load configuration from file, falling back to defaults
    pub fn load(path: Option<&Path>) -> Result<Self> {
        let path = path.map(PathBuf::from).unwrap_or_else(Self::default_path);

        if path.exists() {
            let content = std::fs::read_to_string(&path)
                .with_context(|| format!("Failed to read config file: {}", path.display()))?;
            let config: Config = toml::from_str(&content)
                .with_context(|| format!("Failed to parse config file: {}", path.display()))?;
            Ok(config)
        } else {
            // Create default config file
            let config = Config::default();
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let content = toml::to_string_pretty(&config)?;
            std::fs::write(&path, content)?;
            Ok(config)
        }
    }

    /// Load config and apply CLI overrides
    pub fn load_with_overrides(path: Option<&Path>, overrides: &ConfigOverrides) -> Result<Self> {
        let mut config = Self::load(path)?;
        config.apply_overrides(overrides);
        Ok(config)
    }

    /// Apply CLI overrides to config
    pub fn apply_overrides(&mut self, overrides: &ConfigOverrides) {
        // OMLX overrides
        if let Some(ref endpoint) = overrides.omlx_endpoint {
            self.omlx.endpoint = endpoint.clone();
        }
        if let Some(port) = overrides.omlx_port {
            // Update port in endpoint URL
            if let Ok(mut url) = url::Url::parse(&self.omlx.endpoint) {
                let _ = url.set_port(Some(port));
                self.omlx.endpoint = url.to_string().trim_end_matches('/').to_string();
            }
        }
        if let Some(ref api_key) = overrides.omlx_api_key {
            self.omlx.api_key = Some(api_key.clone());
        }
        if let Some(threshold) = overrides.idle_threshold {
            self.omlx.idle_threshold_secs = threshold;
        }

        // Darkbloom overrides
        if let Some(ref binary) = overrides.darkbloom_binary {
            self.darkbloom.binary_path = binary.clone();
        }
        if let Some(ref model) = overrides.darkbloom_model {
            self.darkbloom.model = model.clone();
        }
        if let Some(ram) = overrides.darkbloom_model_ram {
            self.darkbloom.model_ram_gb = ram;
        }

        // Dashboard overrides
        if let Some(port) = overrides.dashboard_port {
            self.dashboard.port = port;
        }
        if overrides.dashboard_disabled {
            self.dashboard.enabled = false;
        }

        // Memory overrides
        if let Some(mem) = overrides.min_available_memory {
            self.memory.min_available_gb = mem;
        }
    }

    /// Save config to file
    pub fn save(&self, path: Option<&Path>) -> Result<()> {
        let path = path.map(PathBuf::from).unwrap_or_else(Self::default_path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self)?;
        std::fs::write(&path, content)?;
        Ok(())
    }

    /// Set a config value by key path (e.g., "omlx.endpoint")
    pub fn set_value(&mut self, key: &str, value: &str) -> Result<()> {
        match key {
            // OMLX settings
            "omlx.endpoint" => self.omlx.endpoint = value.to_string(),
            "omlx.api_key" => self.omlx.api_key = Some(value.to_string()),
            "omlx.idle_threshold" | "omlx.idle_threshold_secs" => {
                self.omlx.idle_threshold_secs = value
                    .parse()
                    .with_context(|| format!("Invalid number: {}", value))?;
            }
            "omlx.poll_interval" | "omlx.poll_interval_secs" => {
                self.omlx.poll_interval_secs = value
                    .parse()
                    .with_context(|| format!("Invalid number: {}", value))?;
            }
            "omlx.min_idle_polls" => {
                self.omlx.min_idle_polls = value
                    .parse()
                    .with_context(|| format!("Invalid number: {}", value))?;
            }

            // Darkbloom settings
            "darkbloom.binary" | "darkbloom.binary_path" => {
                self.darkbloom.binary_path = value.to_string();
            }
            "darkbloom.model" => self.darkbloom.model = value.to_string(),
            "darkbloom.model_ram" | "darkbloom.model_ram_gb" => {
                self.darkbloom.model_ram_gb = value
                    .parse()
                    .with_context(|| format!("Invalid number: {}", value))?;
            }
            "darkbloom.shutdown_strategy" => {
                self.darkbloom.shutdown_strategy = match value.to_lowercase().as_str() {
                    "graceful" => ShutdownStrategy::Graceful,
                    "immediate" => ShutdownStrategy::Immediate,
                    _ => anyhow::bail!(
                        "Invalid shutdown strategy: {} (use 'graceful' or 'immediate')",
                        value
                    ),
                };
            }

            // Dashboard settings
            "dashboard.enabled" => {
                self.dashboard.enabled = value
                    .parse()
                    .with_context(|| format!("Invalid boolean: {}", value))?;
            }
            "dashboard.port" => {
                self.dashboard.port = value
                    .parse()
                    .with_context(|| format!("Invalid port: {}", value))?;
            }
            "dashboard.bind" => self.dashboard.bind = value.to_string(),

            // Memory settings
            "memory.min_available" | "memory.min_available_gb" => {
                self.memory.min_available_gb = value
                    .parse()
                    .with_context(|| format!("Invalid number: {}", value))?;
            }

            // Daemon settings
            "daemon.log_level" => self.daemon.log_level = value.to_string(),

            _ => anyhow::bail!("Unknown config key: {}", key),
        }
        Ok(())
    }

    /// Get the default configuration file path
    pub fn default_path() -> PathBuf {
        directories::ProjectDirs::from("ai", "darkbloom", "manager")
            .map(|dirs| dirs.config_dir().join("config.toml"))
            .unwrap_or_else(|| {
                dirs::home_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join(".config/dark-bloom-manager/config.toml")
            })
    }

    /// Validate configuration
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();

        if self.omlx.poll_interval_secs == 0 {
            errors.push("omlx.poll_interval_secs must be > 0".to_string());
        }

        if self.omlx.idle_threshold_secs == 0 {
            errors.push("omlx.idle_threshold_secs must be > 0".to_string());
        }

        if self.darkbloom.model_ram_gb <= 0.0 {
            errors.push("darkbloom.model_ram_gb must be > 0".to_string());
        }

        if self.memory.min_available_gb <= 0.0 {
            errors.push("memory.min_available_gb must be > 0".to_string());
        }

        if self.dashboard.port == 0 {
            errors.push("dashboard.port must be > 0".to_string());
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Get the data directory, creating it if necessary
    pub fn data_dir(&self) -> Result<PathBuf> {
        let path = if self.daemon.data_dir.to_string_lossy().starts_with("~/") {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(self.daemon.data_dir.strip_prefix("~/").unwrap())
        } else {
            self.daemon.data_dir.clone()
        };

        std::fs::create_dir_all(&path)?;
        Ok(path)
    }

    /// Get the database path
    pub fn database_path(&self) -> Result<PathBuf> {
        Ok(self.data_dir()?.join("analytics.db"))
    }
}

fn default_data_dir() -> PathBuf {
    directories::ProjectDirs::from("ai", "darkbloom", "manager")
        .map(|dirs| dirs.data_dir().to_path_buf())
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".local/share/dark-bloom-manager")
        })
}
