//! OMLX client and activity monitoring

mod client;
mod monitor;

pub use client::Client;
pub use monitor::{ActivityMonitor, ActivityState};

use serde::{Deserialize, Serialize};

/// Model information from OMLX
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    #[serde(default)]
    pub loaded: bool,
    #[serde(default)]
    pub is_loading: bool,
    #[serde(default)]
    pub estimated_size: u64,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub model_type: String,
    #[serde(default)]
    pub last_access: Option<f64>,
}

/// Server stats from OMLX
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerStats {
    #[serde(default)]
    pub total_requests: u64,
    #[serde(default, rename = "uptime_seconds")]
    pub uptime_secs: f64,
    /// Active models info contains memory usage
    #[serde(default)]
    pub active_models: Option<ActiveModelsInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ActiveModelsInfo {
    #[serde(default)]
    pub total_active_requests: u32,
    #[serde(default)]
    pub model_memory_used: u64,
}

impl ServerStats {
    /// Get active request count from active_models
    pub fn active_requests(&self) -> u32 {
        self.active_models
            .as_ref()
            .map(|m| m.total_active_requests)
            .unwrap_or(0)
    }

    /// Get memory used in GB from active_models
    pub fn memory_used_gb(&self) -> f64 {
        self.active_models
            .as_ref()
            .map(|m| m.model_memory_used as f64 / 1e9)
            .unwrap_or(0.0)
    }
}
