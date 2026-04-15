//! Axum-based dashboard web server

use anyhow::Result;
use axum::{
    extract::State,
    http::StatusCode,
    response::{Html, IntoResponse, Json},
    routing::get,
    Router,
};
use rust_embed::Embed;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

use crate::analytics::Store as AnalyticsStore;
use crate::darkbloom::Controller as DarkbloomController;
use crate::config::Config;
use crate::daemon::DaemonState;
use crate::{get_status, TimePeriod};

#[derive(Embed)]
#[folder = "src/dashboard/static/"]
struct Assets;

/// Shared state for the dashboard
struct AppState {
    config: Arc<RwLock<Config>>,
    daemon_state: Arc<RwLock<DaemonState>>,
}

/// Dashboard web server
pub struct Server {
    config: Arc<RwLock<Config>>,
    daemon_state: Arc<RwLock<DaemonState>>,
}

impl Server {
    /// Create a new dashboard server
    pub fn new(config: Config, daemon_state: Arc<RwLock<DaemonState>>) -> Self {
        Self { 
            config: Arc::new(RwLock::new(config)), 
            daemon_state,
        }
    }

    /// Run the server
    pub async fn run(self) -> Result<()> {
        let config_snapshot = self.config.read().await.clone();
        let state = Arc::new(AppState {
            config: self.config.clone(),
            daemon_state: self.daemon_state,
        });

        let app = Router::new()
            // Dashboard pages
            .route("/", get(redirect_to_dashboard))
            .route("/dashboard", get(dashboard_page))
            // API endpoints
            .route("/api/status", get(api_status))
            .route("/api/analytics", get(api_analytics))
            .route("/api/transitions", get(api_transitions))
            .route("/api/memory-history", get(api_memory_history))
            .route("/api/state-timeline", get(api_state_timeline))
            .route("/api/earnings", get(api_earnings))
            .route("/api/earnings-history", get(api_earnings_history))
            .route("/api/sessions", get(api_sessions))
            .route("/api/config", get(api_config).post(api_update_config))
            .route("/health", get(health_check))
            // Static assets
            .route("/static/*path", get(static_files))
            .with_state(state);

        let bind_addr = format!("{}:{}", config_snapshot.dashboard.bind, config_snapshot.dashboard.port);
        let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
        
        info!("Dashboard server listening on http://{}", bind_addr);
        axum::serve(listener, app).await?;

        Ok(())
    }
}

// Route handlers

async fn redirect_to_dashboard() -> impl IntoResponse {
    axum::response::Redirect::to("/dashboard")
}

async fn dashboard_page() -> impl IntoResponse {
    Html(include_str!("static/index.html"))
}

async fn health_check() -> impl IntoResponse {
    Json(serde_json::json!({ "status": "ok" }))
}

async fn api_status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let config = state.config.read().await;
    match get_status(&config).await {
        Ok(status) => Json(status).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn api_analytics(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let period = TimePeriod::Day; // TODO: get from query params
    let config = state.config.read().await;
    
    match AnalyticsStore::open(&config) {
        Ok(store) => match store.get_summary(period) {
            Ok(summary) => Json(summary).into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response(),
        },
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn api_transitions(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let config = state.config.read().await;
    match AnalyticsStore::open(&config) {
        Ok(store) => match store.get_recent_transitions(50) {
            Ok(transitions) => Json(transitions).into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response(),
        },
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn api_memory_history(
    State(state): State<Arc<AppState>>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let hours: u32 = params
        .get("hours")
        .and_then(|h| h.parse().ok())
        .unwrap_or(24);
    let config = state.config.read().await;

    match AnalyticsStore::open(&config) {
        Ok(store) => match store.get_memory_history(hours) {
            Ok(history) => Json(history).into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response(),
        },
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn api_state_timeline(
    State(state): State<Arc<AppState>>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let hours: u32 = params
        .get("hours")
        .and_then(|h| h.parse().ok())
        .unwrap_or(24);
    let config = state.config.read().await;

    match AnalyticsStore::open(&config) {
        Ok(store) => match store.get_state_timeline(hours) {
            Ok(timeline) => Json(timeline).into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response(),
        },
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn api_earnings(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let config = state.config.read().await;
    
    // Get live earnings from Darkbloom
    let controller = DarkbloomController::new(&config.darkbloom);
    let live_earnings = controller.earnings().await.ok();

    // Get summary from analytics store
    let summary = AnalyticsStore::open(&config)
        .ok()
        .and_then(|store| store.get_earnings_summary().ok());

    Json(serde_json::json!({
        "live": live_earnings,
        "summary": summary
    }))
}

async fn api_earnings_history(
    State(state): State<Arc<AppState>>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let hours: u32 = params
        .get("hours")
        .and_then(|h| h.parse().ok())
        .unwrap_or(168); // Default to 7 days
    let config = state.config.read().await;

    match AnalyticsStore::open(&config) {
        Ok(store) => match store.get_earnings_history(hours) {
            Ok(history) => Json(history).into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response(),
        },
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn api_sessions(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let config = state.config.read().await;
    match AnalyticsStore::open(&config) {
        Ok(store) => match store.get_recent_sessions(20) {
            Ok(sessions) => Json(sessions).into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response(),
        },
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn api_config(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let config = state.config.read().await;
    Json(config.clone())
}

async fn api_update_config(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<Config>,
) -> impl IntoResponse {
    // Validate the new config
    if let Err(errors) = payload.validate() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ 
                "error": "Invalid configuration",
                "details": errors
            })),
        ).into_response();
    }
    
    // Queue the config update in daemon state
    {
        let mut daemon_state = state.daemon_state.write().await;
        daemon_state.queue_config_update(payload.clone());
    }
    
    // Also update our local config reference
    {
        let mut config = state.config.write().await;
        *config = payload.clone();
    }
    
    // Optionally save to disk
    if let Err(e) = payload.save(None) {
        tracing::warn!("Failed to save config to disk during hot-reload: {}", e);
    }
    
    Json(serde_json::json!({ 
        "status": "ok",
        "message": "Configuration queued for hot-reload"
    })).into_response()
}

async fn static_files(
    axum::extract::Path(path): axum::extract::Path<String>,
) -> impl IntoResponse {
    match Assets::get(&path) {
        Some(content) => {
            let mime = mime_guess::from_path(&path).first_or_octet_stream();
            (
                [(axum::http::header::CONTENT_TYPE, mime.as_ref())],
                content.data.to_vec(),
            )
                .into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
