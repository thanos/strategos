use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::errors::AdapterError;
use crate::models::BackendId;

use super::traits::{
    AdapterCapabilities, ExecutionAdapter, ExecutionHandle, ExecutionRequest, ExecutionStatus,
    UsageReport,
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
}

impl ClaudeAdapter {
    pub fn new(config: ClaudeConfig) -> Self {
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
        }
    }

    pub fn config(&self) -> &ClaudeConfig {
        &self.config
    }
}

#[async_trait]
impl ExecutionAdapter for ClaudeAdapter {
    fn id(&self) -> &BackendId {
        &self.backend_id
    }

    fn capabilities(&self) -> &AdapterCapabilities {
        &self.capabilities
    }

    async fn submit(&self, _request: ExecutionRequest) -> Result<ExecutionHandle, AdapterError> {
        Err(AdapterError::Unsupported(
            "Claude adapter not yet implemented — Phase 1 skeleton only".into(),
        ))
    }

    async fn poll(&self, _handle: &ExecutionHandle) -> Result<ExecutionStatus, AdapterError> {
        Err(AdapterError::Unsupported(
            "Claude adapter not yet implemented".into(),
        ))
    }

    async fn cancel(&self, _handle: &ExecutionHandle) -> Result<(), AdapterError> {
        Err(AdapterError::Unsupported(
            "Claude adapter not yet implemented".into(),
        ))
    }

    async fn usage(&self, _handle: &ExecutionHandle) -> Result<UsageReport, AdapterError> {
        Err(AdapterError::Unsupported(
            "Claude adapter not yet implemented".into(),
        ))
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

    #[tokio::test]
    async fn claude_submit_returns_unsupported() {
        use std::collections::HashMap;
        use std::path::PathBuf;

        use crate::adapters::traits::{ExecutionConstraints, ExecutionContext};
        use crate::models::{TaskId, TaskType};

        let adapter = ClaudeAdapter::new(ClaudeConfig::default());
        let request = ExecutionRequest {
            task_id: TaskId::new(),
            task_type: TaskType::Planning,
            prompt: "test".into(),
            context: ExecutionContext {
                project_path: PathBuf::from("/tmp"),
                working_directory: None,
                files: Vec::new(),
                session_id: None,
                metadata: HashMap::new(),
            },
            constraints: ExecutionConstraints::default(),
        };
        let result = adapter.submit(request).await;
        assert!(matches!(result, Err(AdapterError::Unsupported(_))));
    }
}
