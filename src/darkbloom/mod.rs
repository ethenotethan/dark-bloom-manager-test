//! Darkbloom provider control

mod controller;

pub use controller::Controller;

use serde::{Deserialize, Serialize};

/// Status returned by `darkbloom status`
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DarkbloomProcessStatus {
    #[serde(default)]
    pub running: bool,
    #[serde(default)]
    pub connected: bool,
    pub model: Option<String>,
    #[serde(default)]
    pub active_request: bool,
    pub uptime_secs: Option<u64>,
    pub requests_served: Option<u64>,
    pub earnings_usd: Option<f64>,
}

/// Earnings information from `darkbloom earnings`
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EarningsInfo {
    pub total_usd: f64,
    pub today_usd: f64,
    pub this_week_usd: f64,
    pub this_month_usd: f64,
    pub pending_usd: f64,
    pub total_requests: u64,
    pub total_tokens: u64,
}
