//! OMLX activity monitoring

use anyhow::Result;
use chrono::{DateTime, Utc};
use std::time::Instant;
use tracing::{debug, info, warn};

use crate::config::OmlxConfig;
use super::Client;

/// Tracks OMLX activity state over time
#[derive(Debug, Clone)]
pub struct ActivityState {
    pub last_request_time: Option<DateTime<Utc>>,
    pub last_activity_check: Option<Instant>,
    pub active_request_count: u32,
    pub loaded_models: Vec<String>,
    pub memory_used_gb: f64,
    pub consecutive_idle_polls: u32,
    pub consecutive_unreachable: u32,
    pub api_reachable: bool,
}

impl Default for ActivityState {
    fn default() -> Self {
        Self {
            last_request_time: None,
            last_activity_check: None,
            active_request_count: 0,
            loaded_models: Vec::new(),
            memory_used_gb: 0.0,
            consecutive_idle_polls: 0,
            consecutive_unreachable: 0,
            api_reachable: false,
        }
    }
}

impl ActivityState {
    /// Check if OMLX is considered idle based on configuration
    pub fn is_idle(&self, config: &OmlxConfig) -> bool {
        // Must be reachable
        if !self.api_reachable {
            return false;
        }

        // Must have zero active requests
        if self.active_request_count > 0 {
            return false;
        }

        // Must have been idle for threshold duration
        let idle_long_enough = self.last_request_time
            .map(|t| {
                let elapsed = Utc::now().signed_duration_since(t);
                elapsed.num_seconds() as u64 >= config.idle_threshold_secs
            })
            .unwrap_or(true); // No recorded request = idle

        // Must have consistent idle readings
        let stable = self.consecutive_idle_polls >= config.min_idle_polls;

        idle_long_enough && stable
    }

    /// Check if we should consider starting Darkbloom
    pub fn ready_for_darkbloom(&self, config: &OmlxConfig) -> bool {
        self.is_idle(config) && self.loaded_models.is_empty()
    }
}

/// Monitors OMLX activity by polling the admin API
pub struct ActivityMonitor {
    client: Client,
    config: OmlxConfig,
    state: ActivityState,
}

impl ActivityMonitor {
    /// Create a new activity monitor
    pub fn new(config: OmlxConfig) -> Self {
        let client = Client::new(&config);
        Self {
            client,
            config,
            state: ActivityState::default(),
        }
    }

    /// Poll OMLX and update activity state
    pub async fn poll(&mut self) -> Result<&ActivityState> {
        self.state.last_activity_check = Some(Instant::now());

        // Try to get server stats
        match self.client.get_stats().await {
            Ok(stats) => {
                self.state.api_reachable = true;
                self.state.consecutive_unreachable = 0;
                self.state.active_request_count = stats.active_requests;
                self.state.memory_used_gb = stats.memory_used_gb;

                // Update idle tracking
                if stats.active_requests > 0 {
                    self.state.last_request_time = Some(Utc::now());
                    self.state.consecutive_idle_polls = 0;
                } else {
                    self.state.consecutive_idle_polls += 1;
                }
            }
            Err(e) => {
                warn!("Failed to get OMLX stats: {}", e);
                self.state.api_reachable = false;
                self.state.consecutive_unreachable += 1;
                self.state.consecutive_idle_polls = 0;
            }
        }

        // Try to get loaded models
        match self.client.get_models().await {
            Ok(models) => {
                self.state.loaded_models = models
                    .into_iter()
                    .filter(|m| m.loaded)
                    .map(|m| m.id)
                    .collect();
            }
            Err(e) => {
                debug!("Failed to get OMLX models: {}", e);
                // Don't clear loaded_models on error - keep last known state
            }
        }

        debug!(
            "OMLX poll: reachable={}, active_requests={}, loaded_models={}, consecutive_idle={}",
            self.state.api_reachable,
            self.state.active_request_count,
            self.state.loaded_models.len(),
            self.state.consecutive_idle_polls
        );

        Ok(&self.state)
    }

    /// Get current activity state
    pub fn state(&self) -> &ActivityState {
        &self.state
    }

    /// Check if OMLX is currently idle
    pub fn is_idle(&self) -> bool {
        self.state.is_idle(&self.config)
    }

    /// Unload all models in preparation for Darkbloom
    pub async fn unload_all_models(&self) -> Result<Vec<String>> {
        info!("Unloading all OMLX models");
        self.client.unload_all_models().await
    }

    /// Check if any models are currently loaded
    pub fn has_loaded_models(&self) -> bool {
        !self.state.loaded_models.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_activity_state_default_is_not_idle() {
        let state = ActivityState::default();
        let config = OmlxConfig::default();
        // Default state is not reachable, so not idle
        assert!(!state.is_idle(&config));
    }

    #[test]
    fn test_activity_state_with_active_requests() {
        let mut state = ActivityState::default();
        state.api_reachable = true;
        state.active_request_count = 1;
        state.consecutive_idle_polls = 10;

        let config = OmlxConfig::default();
        assert!(!state.is_idle(&config));
    }

    #[test]
    fn test_activity_state_idle() {
        let mut state = ActivityState::default();
        state.api_reachable = true;
        state.active_request_count = 0;
        state.consecutive_idle_polls = 5;
        // No last_request_time means it's been idle "forever"

        let mut config = OmlxConfig::default();
        config.min_idle_polls = 3;

        assert!(state.is_idle(&config));
    }
}
