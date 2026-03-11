use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::errors::AdapterError;
use crate::models::BackendId;

use super::traits::{
    AdapterCapabilities, ExecutionAdapter, ExecutionHandle, ExecutionRequest, ExecutionStatus,
    UsageReport,
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
}

impl OllamaAdapter {
    pub fn new(config: OllamaConfig) -> Self {
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
        }
    }

    pub fn config(&self) -> &OllamaConfig {
        &self.config
    }
}

#[async_trait]
impl ExecutionAdapter for OllamaAdapter {
    fn id(&self) -> &BackendId {
        &self.backend_id
    }

    fn capabilities(&self) -> &AdapterCapabilities {
        &self.capabilities
    }

    async fn submit(&self, _request: ExecutionRequest) -> Result<ExecutionHandle, AdapterError> {
        Err(AdapterError::Unsupported(
            "Ollama adapter not yet implemented — Phase 1 skeleton only".into(),
        ))
    }

    async fn poll(&self, _handle: &ExecutionHandle) -> Result<ExecutionStatus, AdapterError> {
        Err(AdapterError::Unsupported(
            "Ollama adapter not yet implemented".into(),
        ))
    }

    async fn cancel(&self, _handle: &ExecutionHandle) -> Result<(), AdapterError> {
        Err(AdapterError::Unsupported(
            "Ollama adapter not yet implemented".into(),
        ))
    }

    async fn usage(&self, _handle: &ExecutionHandle) -> Result<UsageReport, AdapterError> {
        Err(AdapterError::Unsupported(
            "Ollama adapter not yet implemented".into(),
        ))
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
    }
}
