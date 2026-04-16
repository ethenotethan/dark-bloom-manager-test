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
        // If we've never seen a request, require extra consecutive idle polls
        // to avoid immediately transitioning on startup
        let idle_long_enough = self.last_request_time
            .map(|t| {
                let elapsed = Utc::now().signed_duration_since(t);
                elapsed.num_seconds() as u64 >= config.idle_threshold_secs
            })
            .unwrap_or(false); // No recorded request = NOT idle yet (be conservative)

        // Must have consistent idle readings
        // If we've never seen activity, require idle_threshold_secs / poll_interval polls
        let required_polls = if self.last_request_time.is_none() {
            // On startup with no activity history, wait for threshold duration worth of polls
            std::cmp::max(
                config.min_idle_polls,
                (config.idle_threshold_secs / config.poll_interval_secs) as u32
            )
        } else {
            config.min_idle_polls
        };
        let stable = self.consecutive_idle_polls >= required_polls;

        idle_long_enough || stable
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
                self.state.active_request_count = stats.active_requests();
                self.state.memory_used_gb = stats.memory_used_gb();

                // Update idle tracking
                if stats.active_requests() > 0 {
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

    fn default_config() -> OmlxConfig {
        OmlxConfig {
            idle_threshold_secs: 60,
            poll_interval_secs: 5,
            min_idle_polls: 3,
            ..OmlxConfig::default()
        }
    }

    #[test]
    fn test_activity_state_default_is_not_idle() {
        let state = ActivityState::default();
        let config = default_config();
        // Default state is not reachable, so not idle
        assert!(!state.is_idle(&config));
    }

    #[test]
    fn test_activity_state_not_reachable() {
        let mut state = ActivityState::default();
        state.api_reachable = false;
        state.consecutive_idle_polls = 100; // Many idle polls
        
        let config = default_config();
        assert!(!state.is_idle(&config));
    }

    #[test]
    fn test_activity_state_with_active_requests() {
        let mut state = ActivityState::default();
        state.api_reachable = true;
        state.active_request_count = 1;
        state.consecutive_idle_polls = 10;

        let config = default_config();
        assert!(!state.is_idle(&config));
    }

    #[test]
    fn test_activity_state_not_enough_idle_polls_recent_request() {
        let mut state = ActivityState::default();
        state.api_reachable = true;
        state.active_request_count = 0;
        state.consecutive_idle_polls = 2; // Less than min_idle_polls (3)
        state.last_request_time = Some(Utc::now() - chrono::Duration::seconds(30)); // Recent, within threshold

        let config = default_config();
        // Recent request + not enough polls = not idle
        assert!(!state.is_idle(&config));
    }

    #[test]
    fn test_activity_state_idle_with_old_request() {
        let mut state = ActivityState::default();
        state.api_reachable = true;
        state.active_request_count = 0;
        state.consecutive_idle_polls = 1; // Even with few polls
        state.last_request_time = Some(Utc::now() - chrono::Duration::seconds(120)); // Longer than threshold

        let config = default_config(); // 60 second threshold
        // Old enough request = idle (even without many polls)
        assert!(state.is_idle(&config));
    }

    #[test]
    fn test_activity_state_idle_with_enough_polls() {
        let mut state = ActivityState::default();
        state.api_reachable = true;
        state.active_request_count = 0;
        state.consecutive_idle_polls = 5; // More than min_idle_polls (3)
        state.last_request_time = Some(Utc::now() - chrono::Duration::seconds(30)); // Recent

        let config = default_config();
        // Enough consecutive polls = idle (even with recent request)
        assert!(state.is_idle(&config));
    }

    #[test]
    fn test_activity_state_idle_no_previous_request() {
        // When there's no last_request_time, require more polls
        let mut state = ActivityState::default();
        state.api_reachable = true;
        state.active_request_count = 0;
        state.last_request_time = None; // Never seen a request
        
        let config = default_config();
        
        // With only 3 polls (min_idle_polls), should NOT be idle
        // because we need idle_threshold/poll_interval = 60/5 = 12 polls
        state.consecutive_idle_polls = 3;
        assert!(!state.is_idle(&config));
        
        // With 12 polls, should be idle
        state.consecutive_idle_polls = 12;
        assert!(state.is_idle(&config));
    }

    #[test]
    fn test_activity_state_recent_request_few_polls_not_idle() {
        let mut state = ActivityState::default();
        state.api_reachable = true;
        state.active_request_count = 0;
        state.consecutive_idle_polls = 2; // Less than min_idle_polls
        state.last_request_time = Some(Utc::now() - chrono::Duration::seconds(30)); // Recent

        let config = default_config(); // 60 second threshold
        // Recent request AND few polls = not idle
        assert!(!state.is_idle(&config));
    }

    #[test]
    fn test_ready_for_darkbloom_requires_no_models() {
        let mut state = ActivityState::default();
        state.api_reachable = true;
        state.active_request_count = 0;
        state.consecutive_idle_polls = 20;
        state.last_request_time = Some(Utc::now() - chrono::Duration::seconds(120));

        let config = default_config();

        // With models loaded, not ready
        state.loaded_models = vec!["model1".to_string()];
        assert!(!state.ready_for_darkbloom(&config));

        // Without models, ready
        state.loaded_models = vec![];
        assert!(state.ready_for_darkbloom(&config));
    }

    #[test]
    fn test_activity_state_default_values() {
        let state = ActivityState::default();
        assert!(state.last_request_time.is_none());
        assert!(state.last_activity_check.is_none());
        assert_eq!(state.active_request_count, 0);
        assert!(state.loaded_models.is_empty());
        assert_eq!(state.memory_used_gb, 0.0);
        assert_eq!(state.consecutive_idle_polls, 0);
        assert_eq!(state.consecutive_unreachable, 0);
        assert!(!state.api_reachable);
    }
}
