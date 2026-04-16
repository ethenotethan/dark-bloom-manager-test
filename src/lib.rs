//! Dark Bloom Manager - Supervisor daemon for Darkbloom provider with OMLX coordination
//!
//! This daemon monitors local OMLX inference activity and manages the Darkbloom
//! provider to utilize idle compute for decentralized inference.

pub mod analytics;
pub mod config;
pub mod daemon;
pub mod darkbloom;
pub mod dashboard;
pub mod decision;
pub mod launchd;
pub mod memory;
pub mod omlx;

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::config::Config;

/// System-wide state
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SystemState {
    /// OMLX is actively serving or has loaded models
    OmlxActive,
    /// OMLX is idle, no recent activity
    OmlxIdle,
    /// Transitioning from OMLX to Darkbloom
    UnloadingOmlx,
    /// Starting Darkbloom provider
    StartingDarkbloom,
    /// Darkbloom is actively serving
    DarkbloomActive,
    /// Stopping Darkbloom provider
    StoppingDarkbloom,
    /// Unknown or error state
    #[default]
    Unknown,
}

/// Overall status of the manager
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Status {
    pub daemon: DaemonStatus,
    pub state: SystemState,
    pub omlx: OmlxStatus,
    pub darkbloom: DarkbloomStatus,
    pub memory: MemoryStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonStatus {
    pub running: bool,
    pub uptime_secs: Option<u64>,
    pub pid: Option<u32>,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OmlxStatus {
    pub reachable: bool,
    pub loaded_models: Vec<String>,
    pub memory_gb: f64,
    pub last_request: Option<DateTime<Utc>>,
    pub idle_duration_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DarkbloomStatus {
    pub running: bool,
    pub connected: bool,
    pub model: Option<String>,
    pub uptime_secs: Option<u64>,
    pub requests_served: Option<u64>,
    pub active_request: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryStatus {
    pub system_total_gb: f64,
    pub system_available_gb: f64,
    pub estimated_darkbloom_gb: Option<f64>,
}

/// Analytics summary for a time period
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalyticsSummary {
    pub period: TimePeriod,
    pub omlx_active_pct: f64,
    pub darkbloom_active_pct: f64,
    pub idle_pct: f64,
    pub transitioning_pct: f64,
    pub omlx_requests: u64,
    pub darkbloom_requests_served: u64,
    pub darkbloom_earnings_usd: f64,
    pub transitions_count: u32,
    pub avg_transition_duration_ms: u64,
    pub peak_memory_gb: f64,
    pub avg_memory_gb: f64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TimePeriod {
    Hour,
    Day,
    Week,
    Month,
}

impl std::str::FromStr for TimePeriod {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "hour" => Ok(Self::Hour),
            "day" => Ok(Self::Day),
            "week" => Ok(Self::Week),
            "month" => Ok(Self::Month),
            _ => anyhow::bail!("Invalid time period: {}", s),
        }
    }
}

/// Get current system status
pub async fn get_status(config: &Config) -> Result<Status> {
    let omlx_client = omlx::Client::new(&config.omlx);
    let darkbloom_ctl = darkbloom::Controller::new(&config.darkbloom);
    let mem = memory::get_memory_info()?;

    // Query OMLX
    let omlx_status = match omlx_client.get_models().await {
        Ok(models) => {
            let loaded: Vec<_> = models.iter().filter(|m| m.loaded).collect();
            let memory_gb: f64 = loaded.iter().map(|m| m.estimated_size as f64 / 1e9).sum();
            OmlxStatus {
                reachable: true,
                loaded_models: loaded.iter().map(|m| m.id.clone()).collect(),
                memory_gb,
                last_request: None, // TODO: track this
                idle_duration_secs: None,
            }
        }
        Err(_) => OmlxStatus {
            reachable: false,
            loaded_models: vec![],
            memory_gb: 0.0,
            last_request: None,
            idle_duration_secs: None,
        },
    };

    // Query Darkbloom
    let darkbloom_status = match darkbloom_ctl.status().await {
        Ok(status) => DarkbloomStatus {
            running: status.running,
            connected: status.connected,
            model: status.model,
            uptime_secs: status.uptime_secs,
            requests_served: status.requests_served,
            active_request: status.active_request,
        },
        Err(_) => DarkbloomStatus {
            running: false,
            connected: false,
            model: None,
            uptime_secs: None,
            requests_served: None,
            active_request: false,
        },
    };

    // Determine current state
    let state = if darkbloom_status.running {
        SystemState::DarkbloomActive
    } else if !omlx_status.loaded_models.is_empty() {
        SystemState::OmlxActive
    } else if omlx_status.reachable {
        SystemState::OmlxIdle
    } else {
        SystemState::Unknown
    };

    Ok(Status {
        daemon: DaemonStatus {
            running: true,     // If we're responding, we're running
            uptime_secs: None, // TODO: track daemon start time
            pid: Some(std::process::id()),
            version: env!("CARGO_PKG_VERSION").to_string(),
        },
        state,
        omlx: omlx_status,
        darkbloom: darkbloom_status,
        memory: MemoryStatus {
            system_total_gb: mem.total_gb,
            system_available_gb: mem.available_gb,
            estimated_darkbloom_gb: Some(config.darkbloom.model_ram_gb),
        },
    })
}

/// Get analytics summary for a time period
pub async fn get_analytics(config: &Config, period: &str) -> Result<AnalyticsSummary> {
    let period = period.parse()?;
    let store = analytics::Store::open(config)?;
    store.get_summary(period)
}
