//! Main daemon runner

use anyhow::Result;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::time::interval;
use tracing::{debug, error, info, warn};

use crate::analytics::Store as AnalyticsStore;
use crate::config::Config;
use crate::darkbloom::Controller as DarkbloomController;
use crate::dashboard::Server as DashboardServer;
use crate::decision::{Decision, DecisionEngine};
use crate::memory;
use crate::omlx::ActivityMonitor;
use crate::SystemState;

use super::SignalHandler;

/// Shared daemon state
#[derive(Debug)]
pub struct DaemonState {
    pub current_state: SystemState,
    pub transitioning: bool,
    pub current_session_id: Option<i64>,
    pub session_start_earnings: f64,
}

impl Default for DaemonState {
    fn default() -> Self {
        Self {
            current_state: SystemState::Unknown,
            transitioning: false,
            current_session_id: None,
            session_start_earnings: 0.0,
        }
    }
}

/// The main daemon that orchestrates everything
pub struct Daemon {
    config: Config,
    state: Arc<RwLock<DaemonState>>,
    omlx_monitor: ActivityMonitor,
    darkbloom_ctl: DarkbloomController,
    decision_engine: DecisionEngine,
    analytics: Option<AnalyticsStore>,
}

impl Daemon {
    /// Create a new daemon instance
    pub async fn new(config: Config) -> Result<Self> {
        let omlx_monitor = ActivityMonitor::new(config.omlx.clone());
        let darkbloom_ctl = DarkbloomController::new(&config.darkbloom);
        let decision_engine = DecisionEngine::new(config.clone());
        
        let analytics = if config.analytics.enabled {
            Some(AnalyticsStore::open(&config)?)
        } else {
            None
        };

        Ok(Self {
            config,
            state: Arc::new(RwLock::new(DaemonState::default())),
            omlx_monitor,
            darkbloom_ctl,
            decision_engine,
            analytics,
        })
    }

    /// Run the daemon
    pub async fn run(mut self, foreground: bool) -> Result<()> {
        info!("Starting dark-bloom-manager daemon");

        // Initialize state by checking current system status
        self.initialize_state().await?;

        // Set up signal handling
        let (signal_handler, mut shutdown_rx) = SignalHandler::new();
        let signal_task = tokio::spawn(async move {
            signal_handler.listen().await;
        });

        // Start dashboard server if enabled
        let dashboard_handle = if self.config.dashboard.enabled {
            let server = DashboardServer::new(
                self.config.clone(),
                self.state.clone(),
            );
            Some(tokio::spawn(async move {
                if let Err(e) = server.run().await {
                    error!("Dashboard server error: {}", e);
                }
            }))
        } else {
            None
        };

        // Main loop
        let poll_interval = Duration::from_secs(self.config.omlx.poll_interval_secs);
        let mut ticker = interval(poll_interval);

        info!(
            "Daemon running. Poll interval: {}s, Idle threshold: {}s",
            self.config.omlx.poll_interval_secs,
            self.config.omlx.idle_threshold_secs
        );

        if !foreground {
            info!("Dashboard available at http://{}:{}/dashboard", 
                self.config.dashboard.bind, 
                self.config.dashboard.port
            );
        }

        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    if let Err(e) = self.tick().await {
                        error!("Error in main loop: {}", e);
                    }
                }
                _ = shutdown_rx.recv() => {
                    info!("Shutdown signal received");
                    break;
                }
            }
        }

        // Cleanup
        info!("Shutting down daemon");
        self.cleanup().await?;

        // Cancel background tasks
        signal_task.abort();
        if let Some(handle) = dashboard_handle {
            handle.abort();
        }

        Ok(())
    }

    /// Initialize state by checking what's currently running
    async fn initialize_state(&mut self) -> Result<()> {
        info!("Initializing daemon state");

        // Check Darkbloom status
        let darkbloom_running = self.darkbloom_ctl.is_running().await;

        // Poll OMLX
        let _ = self.omlx_monitor.poll().await;
        let omlx_state = self.omlx_monitor.state();

        // Determine initial state
        let initial_state = if darkbloom_running {
            info!("Found Darkbloom already running");
            SystemState::DarkbloomActive
        } else if !omlx_state.loaded_models.is_empty() {
            info!("Found OMLX with loaded models: {:?}", omlx_state.loaded_models);
            SystemState::OmlxActive
        } else if omlx_state.api_reachable {
            info!("OMLX reachable but idle");
            SystemState::OmlxIdle
        } else {
            warn!("Could not determine initial state");
            SystemState::Unknown
        };

        let mut state = self.state.write().await;
        state.current_state = initial_state;

        Ok(())
    }

    /// Main tick - poll and make decisions
    async fn tick(&mut self) -> Result<()> {
        // Poll OMLX
        let omlx_state = self.omlx_monitor.poll().await?;

        // Get memory info
        let mem_info = memory::get_memory_info()?;

        // Check Darkbloom status
        let darkbloom_running = self.darkbloom_ctl.is_running().await;

        // Get current state
        let current_state = {
            let state = self.state.read().await;
            state.current_state
        };

        // Make decision
        let (decision, reason) = self.decision_engine.evaluate(
            current_state,
            omlx_state,
            darkbloom_running,
            mem_info.available_gb,
        );

        if let Some(reason) = &reason {
            debug!("Decision: {:?}, Reason: {:?}", decision, reason);
        }

        // Execute decision
        match decision {
            Decision::NoOp => {
                // Record snapshot for analytics
                if let Some(ref analytics) = self.analytics {
                    let _ = analytics.record_snapshot(
                        current_state,
                        &omlx_state.loaded_models,
                        omlx_state.memory_used_gb,
                        darkbloom_running,
                        mem_info.available_gb,
                    );

                    // Record earnings if Darkbloom is running
                    if darkbloom_running {
                        if let Ok(earnings) = self.darkbloom_ctl.earnings().await {
                            let state = self.state.read().await;
                            let session_earnings = earnings.total_usd - state.session_start_earnings;
                            let _ = analytics.record_earnings_snapshot(
                                earnings.total_usd,
                                earnings.today_usd,
                                earnings.pending_usd,
                                earnings.total_requests,
                                session_earnings,
                            );
                        }
                    }
                }
            }
            Decision::StartDarkbloomTransition => {
                self.transition_to_darkbloom().await?;
            }
            Decision::StartOmlxTransition => {
                self.transition_to_omlx().await?;
            }
            Decision::ContinueTransition => {
                // Transition in progress, handled by transition functions
            }
        }

        Ok(())
    }

    /// Transition from OMLX to Darkbloom
    async fn transition_to_darkbloom(&mut self) -> Result<()> {
        let start = std::time::Instant::now();

        // Set state to transitioning
        {
            let mut state = self.state.write().await;
            state.current_state = SystemState::UnloadingOmlx;
            state.transitioning = true;
        }

        // Step 1: Unload OMLX models
        info!("Unloading OMLX models");
        let unloaded = self.omlx_monitor.unload_all_models().await?;
        info!("Unloaded {} OMLX models: {:?}", unloaded.len(), unloaded);

        // Step 2: Wait for memory to be available
        let required_memory = self.config.darkbloom.model_ram_gb;
        info!("Waiting for {}GB memory to be available", required_memory);
        
        if !memory::wait_for_memory(required_memory, 30).await? {
            warn!("Memory not freed in time, proceeding anyway");
        }

        // Step 3: Update state and start Darkbloom
        {
            let mut state = self.state.write().await;
            state.current_state = SystemState::StartingDarkbloom;
        }

        info!("Starting Darkbloom provider");
        match self.darkbloom_ctl.start().await {
            Ok(()) => {
                // Get starting earnings for session tracking
                let starting_earnings = self.darkbloom_ctl.earnings().await
                    .map(|e| e.total_usd)
                    .unwrap_or(0.0);

                // Start session tracking
                let session_id = if let Some(ref analytics) = self.analytics {
                    analytics.start_darkbloom_session(&self.config.darkbloom.model).ok()
                } else {
                    None
                };

                let mut state = self.state.write().await;
                state.current_state = SystemState::DarkbloomActive;
                state.transitioning = false;
                state.current_session_id = session_id;
                state.session_start_earnings = starting_earnings;

                let duration = start.elapsed();
                info!("Transition to Darkbloom complete in {:?}", duration);

                // Record transition
                if let Some(ref analytics) = self.analytics {
                    let _ = analytics.record_transition(
                        "OMLX",
                        "DARKBLOOM",
                        "idle_timeout",
                        duration.as_millis() as u64,
                        true,
                    );
                }
            }
            Err(e) => {
                error!("Failed to start Darkbloom: {}", e);
                let mut state = self.state.write().await;
                state.current_state = SystemState::OmlxIdle;
                state.transitioning = false;

                // Record failed transition
                if let Some(ref analytics) = self.analytics {
                    let _ = analytics.record_transition(
                        "OMLX",
                        "DARKBLOOM",
                        "idle_timeout",
                        start.elapsed().as_millis() as u64,
                        false,
                    );
                }
            }
        }

        Ok(())
    }

    /// Transition from Darkbloom back to OMLX
    async fn transition_to_omlx(&mut self) -> Result<()> {
        let start = std::time::Instant::now();

        // Set state to transitioning
        {
            let mut state = self.state.write().await;
            state.current_state = SystemState::StoppingDarkbloom;
            state.transitioning = true;
        }

        // Get final earnings before stopping
        let final_earnings = self.darkbloom_ctl.earnings().await.ok();
        let final_status = self.darkbloom_ctl.status().await.ok();

        // Stop Darkbloom
        info!("Stopping Darkbloom provider");
        match self.darkbloom_ctl.stop().await {
            Ok(()) => {
                // End session tracking
                {
                    let state = self.state.read().await;
                    if let (Some(session_id), Some(ref analytics)) = (state.current_session_id, &self.analytics) {
                        let session_earnings = final_earnings
                            .as_ref()
                            .map(|e| e.total_usd - state.session_start_earnings)
                            .unwrap_or(0.0);
                        let requests = final_status
                            .as_ref()
                            .and_then(|s| s.requests_served)
                            .unwrap_or(0);
                        let _ = analytics.end_darkbloom_session(session_id, requests, session_earnings);
                        info!("Session ended: {} requests, ${:.4} earned", requests, session_earnings);
                    }
                }

                let mut state = self.state.write().await;
                state.current_state = SystemState::OmlxActive;
                state.transitioning = false;
                state.current_session_id = None;
                state.session_start_earnings = 0.0;

                let duration = start.elapsed();
                info!("Transition to OMLX complete in {:?}", duration);

                // Record transition
                if let Some(ref analytics) = self.analytics {
                    let _ = analytics.record_transition(
                        "DARKBLOOM",
                        "OMLX",
                        "omlx_request",
                        duration.as_millis() as u64,
                        true,
                    );
                }
            }
            Err(e) => {
                error!("Failed to stop Darkbloom: {}", e);
                let mut state = self.state.write().await;
                state.transitioning = false;
                // Keep current state, will retry next tick
            }
        }

        Ok(())
    }

    /// Cleanup on shutdown
    async fn cleanup(&mut self) -> Result<()> {
        // Stop Darkbloom if we started it
        let current_state = {
            self.state.read().await.current_state
        };

        if matches!(current_state, SystemState::DarkbloomActive | SystemState::StartingDarkbloom) {
            info!("Stopping Darkbloom before exit");
            let _ = self.darkbloom_ctl.stop().await;
        }

        Ok(())
    }
}
