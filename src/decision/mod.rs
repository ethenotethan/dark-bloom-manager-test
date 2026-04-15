//! Decision engine for state transitions

use tracing::{debug, info};

use crate::config::Config;
use crate::omlx::ActivityState;
use crate::SystemState;

/// Decision about what action to take
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// No action needed, maintain current state
    NoOp,
    /// Start transitioning to Darkbloom
    StartDarkbloomTransition,
    /// Start transitioning back to OMLX
    StartOmlxTransition,
    /// Continue ongoing transition
    ContinueTransition,
}

/// Reasons for decisions (for logging/analytics)
#[derive(Debug, Clone)]
pub enum DecisionReason {
    OmlxIdle { idle_secs: u64 },
    OmlxActive { active_requests: u32 },
    OmlxModelLoaded { model: String },
    DarkbloomRunning,
    InsufficientMemory { available_gb: f64, required_gb: f64 },
    OmlxUnreachable,
    AlreadyInDesiredState,
}

/// Engine that makes decisions about state transitions
pub struct DecisionEngine {
    config: Config,
}

impl DecisionEngine {
    pub fn new(config: Config) -> Self {
        Self { config }
    }

    /// Evaluate current state and decide what to do
    pub fn evaluate(
        &self,
        current_state: SystemState,
        omlx_activity: &ActivityState,
        darkbloom_running: bool,
        available_memory_gb: f64,
    ) -> (Decision, Option<DecisionReason>) {
        debug!(
            "Evaluating: state={:?}, omlx_reachable={}, omlx_idle={}, darkbloom_running={}, available_memory={}GB",
            current_state,
            omlx_activity.api_reachable,
            omlx_activity.is_idle(&self.config.omlx),
            darkbloom_running,
            available_memory_gb
        );

        match current_state {
            SystemState::OmlxActive | SystemState::OmlxIdle => {
                self.evaluate_from_omlx(omlx_activity, darkbloom_running, available_memory_gb)
            }

            SystemState::DarkbloomActive => self.evaluate_from_darkbloom(omlx_activity),

            SystemState::UnloadingOmlx
            | SystemState::StartingDarkbloom
            | SystemState::StoppingDarkbloom => {
                // Transitions in progress - continue them
                (Decision::ContinueTransition, None)
            }

            SystemState::Unknown => {
                // Try to recover by checking actual state
                if darkbloom_running {
                    (Decision::NoOp, Some(DecisionReason::DarkbloomRunning))
                } else if !omlx_activity.loaded_models.is_empty() {
                    (
                        Decision::NoOp,
                        Some(DecisionReason::OmlxModelLoaded {
                            model: omlx_activity
                                .loaded_models
                                .first()
                                .cloned()
                                .unwrap_or_default(),
                        }),
                    )
                } else {
                    (Decision::NoOp, Some(DecisionReason::AlreadyInDesiredState))
                }
            }
        }
    }

    /// Evaluate when in OMLX state
    fn evaluate_from_omlx(
        &self,
        omlx_activity: &ActivityState,
        darkbloom_running: bool,
        available_memory_gb: f64,
    ) -> (Decision, Option<DecisionReason>) {
        // If Darkbloom is somehow running, stop it first
        if darkbloom_running {
            return (
                Decision::StartOmlxTransition,
                Some(DecisionReason::DarkbloomRunning),
            );
        }

        // Check if OMLX is unreachable
        if !omlx_activity.api_reachable {
            match self.config.omlx.unreachable_behavior {
                crate::config::UnreachableBehavior::AssumeActive => {
                    return (Decision::NoOp, Some(DecisionReason::OmlxUnreachable));
                }
                crate::config::UnreachableBehavior::AssumeIdle => {
                    // Fall through to check memory
                }
            }
        }

        // Check if OMLX is idle
        if !omlx_activity.is_idle(&self.config.omlx) {
            let reason = if omlx_activity.active_request_count > 0 {
                DecisionReason::OmlxActive {
                    active_requests: omlx_activity.active_request_count,
                }
            } else if !omlx_activity.loaded_models.is_empty() {
                DecisionReason::OmlxModelLoaded {
                    model: omlx_activity
                        .loaded_models
                        .first()
                        .cloned()
                        .unwrap_or_default(),
                }
            } else {
                DecisionReason::AlreadyInDesiredState
            };
            return (Decision::NoOp, Some(reason));
        }

        // OMLX is idle - check if we have enough memory for Darkbloom
        let required_memory = self.config.darkbloom.model_ram_gb;
        if available_memory_gb < required_memory {
            // Need to unload OMLX models first
            if !omlx_activity.loaded_models.is_empty() {
                info!("OMLX idle but models loaded - starting transition to unload");
                return (
                    Decision::StartDarkbloomTransition,
                    Some(DecisionReason::OmlxIdle {
                        idle_secs: self.config.omlx.idle_threshold_secs,
                    }),
                );
            }
            return (
                Decision::NoOp,
                Some(DecisionReason::InsufficientMemory {
                    available_gb: available_memory_gb,
                    required_gb: required_memory,
                }),
            );
        }

        // Ready to start Darkbloom
        info!("OMLX idle and memory available - starting Darkbloom transition");
        (
            Decision::StartDarkbloomTransition,
            Some(DecisionReason::OmlxIdle {
                idle_secs: self.config.omlx.idle_threshold_secs,
            }),
        )
    }

    /// Evaluate when Darkbloom is running
    fn evaluate_from_darkbloom(
        &self,
        omlx_activity: &ActivityState,
    ) -> (Decision, Option<DecisionReason>) {
        // Check if OMLX has new activity
        if omlx_activity.active_request_count > 0 {
            info!("OMLX has active requests - stopping Darkbloom");
            return (
                Decision::StartOmlxTransition,
                Some(DecisionReason::OmlxActive {
                    active_requests: omlx_activity.active_request_count,
                }),
            );
        }

        // Check if models were loaded (request may have come in)
        if !omlx_activity.loaded_models.is_empty() {
            info!("OMLX models loaded - stopping Darkbloom");
            return (
                Decision::StartOmlxTransition,
                Some(DecisionReason::OmlxModelLoaded {
                    model: omlx_activity
                        .loaded_models
                        .first()
                        .cloned()
                        .unwrap_or_default(),
                }),
            );
        }

        // Keep Darkbloom running
        (Decision::NoOp, Some(DecisionReason::AlreadyInDesiredState))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> Config {
        Config::default()
    }

    fn idle_omlx_state() -> ActivityState {
        ActivityState {
            api_reachable: true,
            active_request_count: 0,
            loaded_models: vec![],
            consecutive_idle_polls: 5,
            ..Default::default()
        }
    }

    #[test]
    fn test_start_darkbloom_when_omlx_idle() {
        let engine = DecisionEngine::new(test_config());
        let omlx = idle_omlx_state();

        let (decision, _) = engine.evaluate(
            SystemState::OmlxIdle,
            &omlx,
            false, // darkbloom not running
            64.0,  // plenty of memory
        );

        assert_eq!(decision, Decision::StartDarkbloomTransition);
    }

    #[test]
    fn test_noop_when_omlx_active() {
        let engine = DecisionEngine::new(test_config());
        let mut omlx = idle_omlx_state();
        omlx.active_request_count = 1;

        let (decision, _) = engine.evaluate(SystemState::OmlxActive, &omlx, false, 64.0);

        assert_eq!(decision, Decision::NoOp);
    }

    #[test]
    fn test_stop_darkbloom_when_omlx_needs_it() {
        let engine = DecisionEngine::new(test_config());
        let mut omlx = idle_omlx_state();
        omlx.active_request_count = 1;

        let (decision, _) = engine.evaluate(SystemState::DarkbloomActive, &omlx, true, 64.0);

        assert_eq!(decision, Decision::StartOmlxTransition);
    }
}
