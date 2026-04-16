//! HTTP client for OMLX admin API

use anyhow::{Context, Result};
use reqwest::{cookie::Jar, Client as HttpClient};
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info, warn};

use super::{ModelInfo, ServerStats};
use crate::config::OmlxConfig;

/// Client for interacting with OMLX admin API
///
/// OMLX uses session-based authentication for admin endpoints.
/// The client logs in with the API key and stores the session cookie.
pub struct Client {
    http: HttpClient,
    /// Client with longer timeout for model operations (load/unload)
    http_model_ops: HttpClient,
    base_url: String,
    api_key: Option<String>,
    logged_in: std::sync::atomic::AtomicBool,
}

impl Client {
    /// Create a new OMLX client
    pub fn new(config: &OmlxConfig) -> Self {
        // Create a cookie jar to store session cookies (shared between clients)
        let cookie_jar = Arc::new(Jar::default());

        let http = HttpClient::builder()
            .timeout(Duration::from_secs(config.request_timeout_secs))
            .cookie_provider(cookie_jar.clone())
            .build()
            .expect("Failed to create HTTP client");

        // Longer timeout for model operations (unload can take 30+ seconds)
        let http_model_ops = HttpClient::builder()
            .timeout(Duration::from_secs(60))
            .cookie_provider(cookie_jar)
            .build()
            .expect("Failed to create HTTP client for model ops");

        debug!(
            "OMLX client configured: endpoint={}, api_key={}",
            config.endpoint,
            if config.api_key.is_some() {
                "set"
            } else {
                "not set"
            }
        );

        Self {
            http,
            http_model_ops,
            base_url: config.endpoint.trim_end_matches('/').to_string(),
            api_key: config.api_key.clone(),
            logged_in: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Login to OMLX admin API to get session cookie
    async fn login(&self) -> Result<()> {
        let api_key = self
            .api_key
            .as_ref()
            .context("OMLX API key not configured")?;

        let url = format!("{}/admin/api/login", self.base_url);
        debug!("Logging into OMLX at {}", url);

        #[derive(serde::Serialize)]
        struct LoginRequest<'a> {
            api_key: &'a str,
        }

        let resp = self
            .http
            .post(&url)
            .json(&LoginRequest { api_key })
            .send()
            .await
            .context("Failed to connect to OMLX for login")?;

        if resp.status().is_success() {
            self.logged_in
                .store(true, std::sync::atomic::Ordering::SeqCst);
            info!("Successfully logged into OMLX admin API");
            Ok(())
        } else {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            warn!("OMLX login failed: {} - {}", status, body);
            anyhow::bail!("OMLX login failed: {} - {}", status, body)
        }
    }

    /// Ensure we're logged in before making admin requests
    async fn ensure_logged_in(&self) -> Result<()> {
        if !self.logged_in.load(std::sync::atomic::Ordering::SeqCst) {
            self.login().await?;
        }
        Ok(())
    }

    /// Check if OMLX server is reachable
    pub async fn health_check(&self) -> Result<bool> {
        let url = format!("{}/health", self.base_url);
        match self.http.get(&url).send().await {
            Ok(resp) => Ok(resp.status().is_success()),
            Err(e) => {
                debug!("OMLX health check failed: {}", e);
                Ok(false)
            }
        }
    }

    /// Get list of all models with their status
    pub async fn get_models(&self) -> Result<Vec<ModelInfo>> {
        self.ensure_logged_in().await?;

        let url = format!("{}/admin/api/models", self.base_url);
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .context("Failed to connect to OMLX")?;

        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            // Session expired, try re-login
            self.logged_in
                .store(false, std::sync::atomic::Ordering::SeqCst);
            self.login().await?;
            // Retry
            let resp = self
                .http
                .get(&url)
                .send()
                .await
                .context("Failed to connect to OMLX")?;
            if !resp.status().is_success() {
                anyhow::bail!("OMLX returned status {}", resp.status());
            }
            return self.parse_models_response(resp).await;
        }

        if !resp.status().is_success() {
            anyhow::bail!("OMLX returned status {}", resp.status());
        }

        self.parse_models_response(resp).await
    }

    async fn parse_models_response(&self, resp: reqwest::Response) -> Result<Vec<ModelInfo>> {
        #[derive(serde::Deserialize)]
        struct Response {
            models: Vec<ModelInfo>,
        }

        let data: Response = resp.json().await.context("Failed to parse OMLX response")?;

        Ok(data.models)
    }

    /// Get server statistics
    pub async fn get_stats(&self) -> Result<ServerStats> {
        self.ensure_logged_in().await?;

        let url = format!("{}/admin/api/stats", self.base_url);
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .context("Failed to connect to OMLX")?;

        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            // Session expired, try re-login
            self.logged_in
                .store(false, std::sync::atomic::Ordering::SeqCst);
            self.login().await?;
            // Retry
            let resp = self
                .http
                .get(&url)
                .send()
                .await
                .context("Failed to connect to OMLX")?;
            if !resp.status().is_success() {
                anyhow::bail!("OMLX returned status {}", resp.status());
            }
            return resp
                .json()
                .await
                .context("Failed to parse OMLX stats response");
        }

        if !resp.status().is_success() {
            anyhow::bail!("OMLX returned status {}", resp.status());
        }

        let stats: ServerStats = resp
            .json()
            .await
            .context("Failed to parse OMLX stats response")?;

        Ok(stats)
    }

    /// Unload a specific model
    /// Uses longer timeout as unload can take 30+ seconds for large models
    pub async fn unload_model(&self, model_id: &str) -> Result<()> {
        self.ensure_logged_in().await?;

        info!("Unloading OMLX model: {} (this may take a while)", model_id);
        let url = format!("{}/admin/api/models/{}/unload", self.base_url, model_id);

        // Use model_ops client with longer timeout
        let resp = self
            .http_model_ops
            .post(&url)
            .send()
            .await
            .context("Failed to connect to OMLX for model unload")?;

        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            // Session expired, try re-login
            self.logged_in
                .store(false, std::sync::atomic::Ordering::SeqCst);
            self.login().await?;
            // Retry with model_ops client
            let resp = self
                .http_model_ops
                .post(&url)
                .send()
                .await
                .context("Failed to connect to OMLX for model unload")?;
            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                anyhow::bail!("Failed to unload model {}: {} - {}", model_id, status, body);
            }
            info!("Unloaded OMLX model: {}", model_id);
            return Ok(());
        }

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Failed to unload model {}: {} - {}", model_id, status, body);
        }

        info!("Unloaded OMLX model: {}", model_id);
        Ok(())
    }

    /// Unload all loaded models
    pub async fn unload_all_models(&self) -> Result<Vec<String>> {
        let models = self.get_models().await?;
        let loaded: Vec<_> = models.into_iter().filter(|m| m.loaded).collect();

        let mut unloaded = Vec::new();
        for model in loaded {
            match self.unload_model(&model.id).await {
                Ok(()) => {
                    unloaded.push(model.id);
                }
                Err(e) => {
                    warn!("Failed to unload model {}: {}", model.id, e);
                }
            }
        }

        Ok(unloaded)
    }

    /// Get count of currently active requests
    pub async fn active_request_count(&self) -> Result<u32> {
        let stats = self.get_stats().await?;
        Ok(stats.active_requests())
    }

    /// Check if any models are currently loaded
    pub async fn has_loaded_models(&self) -> Result<bool> {
        let models = self.get_models().await?;
        Ok(models.iter().any(|m| m.loaded))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_creation() {
        let config = OmlxConfig::default();
        let client = Client::new(&config);
        assert_eq!(client.base_url, "http://localhost:8000");
    }
}
