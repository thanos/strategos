use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::Utc;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::{debug, instrument};

use crate::errors::AdapterError;
use crate::models::{BackendId, MoneyAmount};

use super::traits::{
    AdapterCapabilities, ExecutionAdapter, ExecutionHandle, ExecutionRequest, ExecutionResult,
    ExecutionStatus, HealthStatus, UsageReport,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaConfig {
    pub endpoint: String,
    pub model: String,
    pub timeout_secs: u64,
}

impl Default for OllamaConfig {
    fn default() -> Self {
        Self {
            endpoint: "http://localhost:11434".into(),
            model: "llama3".into(),
            timeout_secs: 300,
        }
    }
}

pub struct OllamaAdapter {
    backend_id: BackendId,
    config: OllamaConfig,
    capabilities: AdapterCapabilities,
    client: Client,
}

impl OllamaAdapter {
    pub fn new(config: OllamaConfig) -> Self {
        let timeout = Duration::from_secs(config.timeout_secs);
        let client = Client::builder()
            .timeout(timeout)
            .build()
            .unwrap_or_default();

        Self {
            backend_id: BackendId::new("ollama"),
            config,
            capabilities: AdapterCapabilities {
                code_editing: false,
                shell_tool_use: false,
                multi_step_agent: false,
                local_execution: true,
                structured_output: false,
                streaming: true,
                subagents: false,
                session_resume: false,
            },
            client,
        }
    }

    pub fn config(&self) -> &OllamaConfig {
        &self.config
    }

    /// Check if the Ollama server is reachable.
    pub async fn is_available(&self) -> bool {
        let url = format!("{}/api/tags", self.config.endpoint);
        match self.client.get(&url).send().await {
            Ok(resp) => resp.status().is_success(),
            Err(_) => false,
        }
    }

    fn generate_url(&self) -> String {
        format!("{}/api/generate", self.config.endpoint)
    }
}

/// Ollama /api/generate request body.
#[derive(Debug, Serialize)]
struct OllamaGenerateRequest {
    model: String,
    prompt: String,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
}

/// Ollama /api/generate response body (non-streaming).
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct OllamaGenerateResponse {
    #[serde(default)]
    response: String,
    #[serde(default)]
    done: bool,
    #[serde(default)]
    total_duration: u64,
    #[serde(default)]
    prompt_eval_count: Option<u64>,
    #[serde(default)]
    eval_count: Option<u64>,
}

#[async_trait]
impl ExecutionAdapter for OllamaAdapter {
    fn id(&self) -> &BackendId {
        &self.backend_id
    }

    fn capabilities(&self) -> &AdapterCapabilities {
        &self.capabilities
    }

    async fn health_check(&self) -> HealthStatus {
        let url = format!("{}/api/tags", self.config.endpoint);
        match self.client.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => HealthStatus::Healthy,
            Ok(resp) => HealthStatus::Degraded(format!("API returned {}", resp.status())),
            Err(e) if e.is_connect() => {
                HealthStatus::Unavailable(format!(
                    "cannot connect to Ollama at {}",
                    self.config.endpoint
                ))
            }
            Err(e) if e.is_timeout() => {
                HealthStatus::Degraded("timeout during health check".into())
            }
            Err(e) => HealthStatus::Unavailable(e.to_string()),
        }
    }

    #[instrument(skip(self, request), fields(backend = "ollama", model = %self.config.model))]
    async fn submit(&self, request: ExecutionRequest) -> Result<ExecutionHandle, AdapterError> {
        let start = Instant::now();

        let system_prompt = format!(
            "You are assisting with a {} task.",
            format!("{:?}", request.task_type)
        );

        let ollama_request = OllamaGenerateRequest {
            model: self.config.model.clone(),
            prompt: request.prompt.clone(),
            stream: false,
            system: Some(system_prompt),
        };

        debug!(
            prompt_length = request.prompt.len(),
            "sending request to Ollama"
        );

        let response = self
            .client
            .post(&self.generate_url())
            .json(&ollama_request)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    AdapterError::Timeout(Duration::from_secs(self.config.timeout_secs))
                } else if e.is_connect() {
                    AdapterError::Unavailable(format!(
                        "cannot connect to Ollama at {}: {}",
                        self.config.endpoint, e
                    ))
                } else {
                    AdapterError::RequestFailed(e.to_string())
                }
            })?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(AdapterError::RequestFailed(format!(
                "Ollama returned {}: {}",
                status, body
            )));
        }

        let ollama_response: OllamaGenerateResponse =
            response.json().await.map_err(|e| {
                AdapterError::RequestFailed(format!("failed to parse Ollama response: {}", e))
            })?;

        let duration = start.elapsed();
        let input_tokens = ollama_response.prompt_eval_count.unwrap_or(0);
        let output_tokens = ollama_response.eval_count.unwrap_or(0);

        debug!(
            input_tokens,
            output_tokens,
            duration_ms = duration.as_millis() as u64,
            "Ollama request completed"
        );

        // Store the result in the handle_id as a serialized payload.
        // This is a simplification — a production system would use a result cache.
        let result = ExecutionResult {
            output: ollama_response.response,
            structured_output: None,
            files_modified: Vec::new(),
            usage: UsageReport {
                input_tokens,
                output_tokens,
                total_tokens: input_tokens + output_tokens,
                cost: MoneyAmount::ZERO, // Local execution, no cost
                model: Some(self.config.model.clone()),
                duration,
            },
            completed_at: Utc::now(),
        };

        let result_json = serde_json::to_string(&result)
            .map_err(|e| AdapterError::Internal(e.to_string()))?;

        Ok(ExecutionHandle {
            backend_id: self.backend_id.clone(),
            handle_id: result_json,
            submitted_at: Utc::now(),
        })
    }

    async fn poll(&self, handle: &ExecutionHandle) -> Result<ExecutionStatus, AdapterError> {
        // Since Ollama generate is synchronous (non-streaming), the result is
        // already available in the handle from submit().
        let result: ExecutionResult = serde_json::from_str(&handle.handle_id)
            .map_err(|e| AdapterError::Internal(format!("corrupt handle: {}", e)))?;
        Ok(ExecutionStatus::Completed(result))
    }

    async fn cancel(&self, _handle: &ExecutionHandle) -> Result<(), AdapterError> {
        // Ollama generate is synchronous — cancellation not supported
        Ok(())
    }

    async fn usage(&self, handle: &ExecutionHandle) -> Result<UsageReport, AdapterError> {
        let result: ExecutionResult = serde_json::from_str(&handle.handle_id)
            .map_err(|e| AdapterError::Internal(format!("corrupt handle: {}", e)))?;
        Ok(result.usage)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ollama_capabilities() {
        let adapter = OllamaAdapter::new(OllamaConfig::default());
        let caps = adapter.capabilities();
        assert!(!caps.code_editing);
        assert!(!caps.shell_tool_use);
        assert!(!caps.multi_step_agent);
        assert!(caps.local_execution);
        assert!(!caps.structured_output);
        assert!(caps.streaming);
        assert!(!caps.subagents);
        assert!(!caps.session_resume);
    }

    #[test]
    fn ollama_id() {
        let adapter = OllamaAdapter::new(OllamaConfig::default());
        assert_eq!(adapter.id().as_str(), "ollama");
    }

    #[test]
    fn ollama_default_config() {
        let config = OllamaConfig::default();
        assert!(config.endpoint.contains("11434"));
        assert_eq!(config.model, "llama3");
        assert_eq!(config.timeout_secs, 300);
    }

    #[test]
    fn generate_url() {
        let adapter = OllamaAdapter::new(OllamaConfig::default());
        assert_eq!(adapter.generate_url(), "http://localhost:11434/api/generate");
    }

    #[tokio::test]
    async fn ollama_unavailable_returns_error() {
        // Point at a port that's (almost certainly) not running Ollama
        let config = OllamaConfig {
            endpoint: "http://127.0.0.1:19999".into(),
            model: "test".into(),
            timeout_secs: 2,
        };
        let adapter = OllamaAdapter::new(config);
        assert!(!adapter.is_available().await);
    }

    #[test]
    fn ollama_response_deserialization() {
        let json = r#"{
            "model": "llama3",
            "response": "Hello world",
            "done": true,
            "total_duration": 1000000,
            "prompt_eval_count": 10,
            "eval_count": 5
        }"#;
        let resp: OllamaGenerateResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.response, "Hello world");
        assert!(resp.done);
        assert_eq!(resp.prompt_eval_count, Some(10));
        assert_eq!(resp.eval_count, Some(5));
    }

    #[test]
    fn ollama_response_with_missing_fields() {
        let json = r#"{"response": "hi", "done": true}"#;
        let resp: OllamaGenerateResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.response, "hi");
        assert_eq!(resp.prompt_eval_count, None);
        assert_eq!(resp.eval_count, None);
    }
}
