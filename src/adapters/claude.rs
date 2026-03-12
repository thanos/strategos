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
pub struct ClaudeConfig {
    pub api_key_env: String,
    pub model: String,
    pub endpoint: String,
    pub max_tokens: Option<u64>,
}

impl Default for ClaudeConfig {
    fn default() -> Self {
        Self {
            api_key_env: "ANTHROPIC_API_KEY".into(),
            model: "claude-sonnet-4-20250514".into(),
            endpoint: "https://api.anthropic.com".into(),
            max_tokens: None,
        }
    }
}

pub struct ClaudeAdapter {
    backend_id: BackendId,
    config: ClaudeConfig,
    capabilities: AdapterCapabilities,
    client: Client,
}

impl ClaudeAdapter {
    pub fn new(config: ClaudeConfig) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .unwrap_or_default();

        Self {
            backend_id: BackendId::new("claude"),
            config,
            capabilities: AdapterCapabilities {
                code_editing: true,
                shell_tool_use: true,
                multi_step_agent: true,
                local_execution: false,
                structured_output: true,
                streaming: true,
                subagents: true,
                session_resume: true,
            },
            client,
        }
    }

    pub fn config(&self) -> &ClaudeConfig {
        &self.config
    }

    fn messages_url(&self) -> String {
        format!("{}/v1/messages", self.config.endpoint)
    }

    fn api_key(&self) -> Result<String, AdapterError> {
        std::env::var(&self.config.api_key_env).map_err(|_| {
            AdapterError::AuthenticationFailed(format!(
                "environment variable '{}' not set",
                self.config.api_key_env
            ))
        })
    }
}

// -----------------------------------------------------------------------
// Anthropic Messages API types
// -----------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct AnthropicRequest {
    model: String,
    max_tokens: u64,
    messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct AnthropicMessage {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct AnthropicResponse {
    id: String,
    content: Vec<AnthropicContent>,
    model: String,
    usage: AnthropicUsage,
    #[serde(default)]
    stop_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct AnthropicContent {
    #[serde(rename = "type")]
    content_type: String,
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AnthropicUsage {
    input_tokens: u64,
    output_tokens: u64,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct AnthropicError {
    #[serde(rename = "type")]
    error_type: String,
    error: AnthropicErrorDetail,
}

#[derive(Debug, Deserialize)]
struct AnthropicErrorDetail {
    #[serde(rename = "type")]
    error_type: String,
    message: String,
}

/// Estimate cost in cents based on model and token counts.
/// Pricing as of mid-2025 (approximate).
pub fn estimate_cost(model: &str, input_tokens: u64, output_tokens: u64) -> MoneyAmount {
    let (input_price_per_mtok, output_price_per_mtok) = match model {
        m if m.contains("opus") => (15.0, 75.0),
        m if m.contains("sonnet") => (3.0, 15.0),
        m if m.contains("haiku") => (0.25, 1.25),
        _ => (3.0, 15.0), // Default to Sonnet pricing
    };

    let input_cost = input_tokens as f64 * input_price_per_mtok / 1_000_000.0;
    let output_cost = output_tokens as f64 * output_price_per_mtok / 1_000_000.0;
    let total_dollars = input_cost + output_cost;

    MoneyAmount::from_dollars(total_dollars)
}

#[async_trait]
impl ExecutionAdapter for ClaudeAdapter {
    fn id(&self) -> &BackendId {
        &self.backend_id
    }

    fn capabilities(&self) -> &AdapterCapabilities {
        &self.capabilities
    }

    async fn health_check(&self) -> HealthStatus {
        match self.api_key() {
            Err(_) => {
                return HealthStatus::Unavailable(format!(
                    "API key not set (env: {})",
                    self.config.api_key_env
                ));
            }
            Ok(key) => {
                // Lightweight check: send a minimal request to verify auth
                let url = format!("{}/v1/messages", self.config.endpoint);
                match self
                    .client
                    .post(&url)
                    .header("x-api-key", &key)
                    .header("anthropic-version", "2023-06-01")
                    .header("content-type", "application/json")
                    .body(r#"{"model":"claude-haiku-4-5-20251001","max_tokens":1,"messages":[{"role":"user","content":"ping"}]}"#)
                    .send()
                    .await
                {
                    Ok(resp) => {
                        let status = resp.status();
                        if status.is_success() {
                            HealthStatus::Healthy
                        } else if status == reqwest::StatusCode::UNAUTHORIZED
                            || status == reqwest::StatusCode::FORBIDDEN
                        {
                            HealthStatus::Unavailable(format!("authentication failed ({})", status))
                        } else if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                            HealthStatus::Degraded("rate limited".into())
                        } else {
                            HealthStatus::Degraded(format!("API returned {}", status))
                        }
                    }
                    Err(e) => {
                        if e.is_connect() {
                            HealthStatus::Unavailable(format!("cannot connect: {}", e))
                        } else if e.is_timeout() {
                            HealthStatus::Degraded("timeout during health check".into())
                        } else {
                            HealthStatus::Unavailable(e.to_string())
                        }
                    }
                }
            }
        }
    }

    #[instrument(skip(self, request), fields(backend = "claude", model = %self.config.model))]
    async fn submit(&self, request: ExecutionRequest) -> Result<ExecutionHandle, AdapterError> {
        let api_key = self.api_key()?;
        let start = Instant::now();

        let max_tokens = self
            .config
            .max_tokens
            .or(request.constraints.max_tokens)
            .unwrap_or(4096);

        let system_prompt = format!(
            "You are assisting with a {} task for a software project.",
            format!("{:?}", request.task_type)
        );

        let anthropic_request = AnthropicRequest {
            model: self.config.model.clone(),
            max_tokens,
            messages: vec![AnthropicMessage {
                role: "user".into(),
                content: request.prompt.clone(),
            }],
            system: Some(system_prompt),
        };

        debug!(
            prompt_length = request.prompt.len(),
            max_tokens,
            "sending request to Claude API"
        );

        let response = self
            .client
            .post(&self.messages_url())
            .header("x-api-key", &api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&anthropic_request)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    AdapterError::Timeout(Duration::from_secs(120))
                } else if e.is_connect() {
                    AdapterError::Unavailable(format!("cannot connect to Claude API: {}", e))
                } else {
                    AdapterError::RequestFailed(e.to_string())
                }
            })?;

        let status = response.status();

        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            let body = response.text().await.unwrap_or_default();
            return Err(AdapterError::AuthenticationFailed(format!(
                "API authentication failed ({}): {}",
                status, body
            )));
        }

        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            let retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .map(Duration::from_secs);
            return Err(AdapterError::RateLimited { retry_after });
        }

        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            // Try to parse as Anthropic error
            if let Ok(err) = serde_json::from_str::<AnthropicError>(&body) {
                return Err(AdapterError::RequestFailed(format!(
                    "{}: {}",
                    err.error.error_type, err.error.message
                )));
            }
            return Err(AdapterError::RequestFailed(format!(
                "Claude API returned {}: {}",
                status, body
            )));
        }

        let anthropic_response: AnthropicResponse =
            response.json().await.map_err(|e| {
                AdapterError::RequestFailed(format!("failed to parse Claude response: {}", e))
            })?;

        let duration = start.elapsed();

        // Extract text from content blocks
        let output: String = anthropic_response
            .content
            .iter()
            .filter_map(|c| c.text.as_deref())
            .collect::<Vec<_>>()
            .join("\n");

        let input_tokens = anthropic_response.usage.input_tokens;
        let output_tokens = anthropic_response.usage.output_tokens;
        let cost = estimate_cost(&anthropic_response.model, input_tokens, output_tokens);

        debug!(
            input_tokens,
            output_tokens,
            cost_cents = cost.cents,
            duration_ms = duration.as_millis() as u64,
            "Claude request completed"
        );

        let result = ExecutionResult {
            output,
            structured_output: None,
            files_modified: Vec::new(),
            usage: UsageReport {
                input_tokens,
                output_tokens,
                total_tokens: input_tokens + output_tokens,
                cost,
                model: Some(anthropic_response.model),
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
        // Messages API is synchronous — result is in the handle from submit()
        let result: ExecutionResult = serde_json::from_str(&handle.handle_id)
            .map_err(|e| AdapterError::Internal(format!("corrupt handle: {}", e)))?;
        Ok(ExecutionStatus::Completed(result))
    }

    async fn cancel(&self, _handle: &ExecutionHandle) -> Result<(), AdapterError> {
        // Messages API doesn't support cancellation
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
    fn claude_capabilities() {
        let adapter = ClaudeAdapter::new(ClaudeConfig::default());
        let caps = adapter.capabilities();
        assert!(caps.code_editing);
        assert!(caps.shell_tool_use);
        assert!(caps.multi_step_agent);
        assert!(!caps.local_execution);
        assert!(caps.structured_output);
        assert!(caps.streaming);
        assert!(caps.subagents);
        assert!(caps.session_resume);
    }

    #[test]
    fn claude_id() {
        let adapter = ClaudeAdapter::new(ClaudeConfig::default());
        assert_eq!(adapter.id().as_str(), "claude");
    }

    #[test]
    fn claude_default_config() {
        let config = ClaudeConfig::default();
        assert_eq!(config.api_key_env, "ANTHROPIC_API_KEY");
        assert!(config.endpoint.starts_with("https://"));
    }

    #[test]
    fn messages_url() {
        let adapter = ClaudeAdapter::new(ClaudeConfig::default());
        assert_eq!(
            adapter.messages_url(),
            "https://api.anthropic.com/v1/messages"
        );
    }

    #[test]
    fn api_key_missing_returns_auth_error() {
        // Ensure the env var is not set for this test
        let config = ClaudeConfig {
            api_key_env: "STRATEGOS_TEST_NONEXISTENT_KEY_12345".into(),
            ..ClaudeConfig::default()
        };
        let adapter = ClaudeAdapter::new(config);
        assert!(matches!(
            adapter.api_key(),
            Err(AdapterError::AuthenticationFailed(_))
        ));
    }

    #[test]
    fn cost_estimation_sonnet() {
        let cost = estimate_cost("claude-sonnet-4-20250514", 1000, 500);
        // Sonnet: $3/MTok input, $15/MTok output
        // 1000 * 3/1M + 500 * 15/1M = 0.003 + 0.0075 = $0.0105
        // = ~1 cent
        assert!(cost.cents >= 1);
        assert!(cost.cents <= 2);
    }

    #[test]
    fn cost_estimation_opus() {
        let cost = estimate_cost("claude-opus-4-20250514", 10000, 5000);
        // Opus: $15/MTok input, $75/MTok output
        // 10000 * 15/1M + 5000 * 75/1M = 0.15 + 0.375 = $0.525
        // = 53 cents
        assert!(cost.cents >= 50);
        assert!(cost.cents <= 55);
    }

    #[test]
    fn cost_estimation_haiku() {
        let cost = estimate_cost("claude-haiku-4-5-20251001", 100000, 50000);
        // Haiku: $0.25/MTok input, $1.25/MTok output
        // 100000 * 0.25/1M + 50000 * 1.25/1M = 0.025 + 0.0625 = $0.0875
        // = 9 cents
        assert!(cost.cents >= 8);
        assert!(cost.cents <= 10);
    }

    #[test]
    fn anthropic_response_deserialization() {
        let json = r#"{
            "id": "msg_123",
            "type": "message",
            "role": "assistant",
            "content": [
                {"type": "text", "text": "Hello, world!"}
            ],
            "model": "claude-sonnet-4-20250514",
            "stop_reason": "end_turn",
            "usage": {
                "input_tokens": 100,
                "output_tokens": 50
            }
        }"#;
        let resp: AnthropicResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.id, "msg_123");
        assert_eq!(resp.content.len(), 1);
        assert_eq!(resp.content[0].text.as_deref(), Some("Hello, world!"));
        assert_eq!(resp.usage.input_tokens, 100);
        assert_eq!(resp.usage.output_tokens, 50);
    }

    #[test]
    fn anthropic_error_deserialization() {
        let json = r#"{
            "type": "error",
            "error": {
                "type": "authentication_error",
                "message": "invalid x-api-key"
            }
        }"#;
        let err: AnthropicError = serde_json::from_str(json).unwrap();
        assert_eq!(err.error.error_type, "authentication_error");
        assert_eq!(err.error.message, "invalid x-api-key");
    }

    #[test]
    fn multiple_content_blocks() {
        let json = r#"{
            "id": "msg_456",
            "type": "message",
            "role": "assistant",
            "content": [
                {"type": "text", "text": "Part 1"},
                {"type": "text", "text": "Part 2"}
            ],
            "model": "claude-sonnet-4-20250514",
            "usage": {"input_tokens": 50, "output_tokens": 30}
        }"#;
        let resp: AnthropicResponse = serde_json::from_str(json).unwrap();
        let output: String = resp
            .content
            .iter()
            .filter_map(|c| c.text.as_deref())
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(output, "Part 1\nPart 2");
    }
}
