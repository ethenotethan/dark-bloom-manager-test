//! HTTP client for OMLX admin API

use anyhow::{Context, Result};
use reqwest::{Client as HttpClient, RequestBuilder};
use std::time::Duration;
use tracing::{debug, warn};

use crate::config::OmlxConfig;
use super::{ModelInfo, ServerStats};

/// Client for interacting with OMLX admin API
pub struct Client {
    http: HttpClient,
    base_url: String,
    api_key: Option<String>,
}

impl Client {
    /// Create a new OMLX client
    pub fn new(config: &OmlxConfig) -> Self {
        let http = HttpClient::builder()
            .timeout(Duration::from_secs(config.request_timeout_secs))
            .build()
            .expect("Failed to create HTTP client");

        Self {
            http,
            base_url: config.endpoint.trim_end_matches('/').to_string(),
            api_key: config.api_key.clone(),
        }
    }
    
    /// Add authorization header if API key is configured
    fn authorize(&self, request: RequestBuilder) -> RequestBuilder {
        if let Some(ref api_key) = self.api_key {
            request.header("Authorization", format!("Bearer {}", api_key))
        } else {
            request
        }
    }

    /// Check if OMLX server is reachable
    pub async fn health_check(&self) -> Result<bool> {
        let url = format!("{}/health", self.base_url);
        let request = self.authorize(self.http.get(&url));
        match request.send().await {
            Ok(resp) => Ok(resp.status().is_success()),
            Err(e) => {
                debug!("OMLX health check failed: {}", e);
                Ok(false)
            }
        }
    }

    /// Get list of all models with their status
    pub async fn get_models(&self) -> Result<Vec<ModelInfo>> {
        let url = format!("{}/admin/api/models", self.base_url);
        let request = self.authorize(self.http.get(&url));
        let resp = request.send().await
            .context("Failed to connect to OMLX")?;

        if !resp.status().is_success() {
            anyhow::bail!("OMLX returned status {}", resp.status());
        }

        #[derive(serde::Deserialize)]
        struct Response {
            models: Vec<ModelInfo>,
        }

        let data: Response = resp.json().await
            .context("Failed to parse OMLX response")?;
        
        Ok(data.models)
    }

    /// Get server statistics
    pub async fn get_stats(&self) -> Result<ServerStats> {
        let url = format!("{}/admin/api/stats", self.base_url);
        let request = self.authorize(self.http.get(&url));
        let resp = request.send().await
            .context("Failed to connect to OMLX")?;

        if !resp.status().is_success() {
            anyhow::bail!("OMLX returned status {}", resp.status());
        }

        let stats: ServerStats = resp.json().await
            .context("Failed to parse OMLX stats response")?;
        
        Ok(stats)
    }

    /// Unload a specific model
    pub async fn unload_model(&self, model_id: &str) -> Result<()> {
        let url = format!("{}/admin/api/models/{}/unload", self.base_url, model_id);
        let request = self.authorize(self.http.post(&url));
        let resp = request.send().await
            .context("Failed to connect to OMLX")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Failed to unload model {}: {} - {}", model_id, status, body);
        }

        debug!("Unloaded OMLX model: {}", model_id);
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
        Ok(stats.active_requests)
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
