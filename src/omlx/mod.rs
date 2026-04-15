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
    pub loaded: bool,
    pub is_loading: bool,
    pub estimated_size: u64,
    pub pinned: bool,
    pub model_type: String,
    pub last_access: Option<String>,
}

/// Server stats from OMLX
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerStats {
    pub active_requests: u32,
    pub total_requests: u64,
    pub memory_used_gb: f64,
    pub uptime_secs: u64,
}
