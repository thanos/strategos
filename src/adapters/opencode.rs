use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::errors::AdapterError;
use crate::models::BackendId;

use super::traits::{
    AdapterCapabilities, ExecutionAdapter, ExecutionHandle, ExecutionRequest, ExecutionStatus,
    HealthStatus, UsageReport,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenCodeConfig {
    pub binary_path: Option<String>,
    pub config_path: Option<String>,
}

impl Default for OpenCodeConfig {
    fn default() -> Self {
        Self {
            binary_path: None,
            config_path: None,
        }
    }
}

pub struct OpenCodeAdapter {
    backend_id: BackendId,
    config: OpenCodeConfig,
    capabilities: AdapterCapabilities,
}

impl OpenCodeAdapter {
    pub fn new(config: OpenCodeConfig) -> Self {
        Self {
            backend_id: BackendId::new("opencode"),
            config,
            capabilities: AdapterCapabilities {
                code_editing: true,
                shell_tool_use: true,
                multi_step_agent: true,
                local_execution: false,
                structured_output: true,
                streaming: true,
                subagents: false,
                session_resume: false,
            },
        }
    }

    pub fn config(&self) -> &OpenCodeConfig {
        &self.config
    }
}

#[async_trait]
impl ExecutionAdapter for OpenCodeAdapter {
    fn id(&self) -> &BackendId {
        &self.backend_id
    }

    fn capabilities(&self) -> &AdapterCapabilities {
        &self.capabilities
    }

    async fn health_check(&self) -> HealthStatus {
        match &self.config.binary_path {
            Some(path) => {
                if std::path::Path::new(path).exists() {
                    HealthStatus::Healthy
                } else {
                    HealthStatus::Unavailable(format!("binary not found at {}", path))
                }
            }
            None => {
                HealthStatus::Unavailable(
                    "opencode adapter is a stub — not yet implemented".into(),
                )
            }
        }
    }

    async fn submit(&self, _request: ExecutionRequest) -> Result<ExecutionHandle, AdapterError> {
        Err(AdapterError::Unsupported(
            "OpenCode adapter is a stub — not yet implemented".into(),
        ))
    }

    async fn poll(&self, _handle: &ExecutionHandle) -> Result<ExecutionStatus, AdapterError> {
        Err(AdapterError::Unsupported(
            "OpenCode adapter is a stub".into(),
        ))
    }

    async fn cancel(&self, _handle: &ExecutionHandle) -> Result<(), AdapterError> {
        Err(AdapterError::Unsupported(
            "OpenCode adapter is a stub".into(),
        ))
    }

    async fn usage(&self, _handle: &ExecutionHandle) -> Result<UsageReport, AdapterError> {
        Err(AdapterError::Unsupported(
            "OpenCode adapter is a stub".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opencode_capabilities() {
        let adapter = OpenCodeAdapter::new(OpenCodeConfig::default());
        let caps = adapter.capabilities();
        assert!(caps.code_editing);
        assert!(caps.shell_tool_use);
        assert!(caps.multi_step_agent);
        assert!(!caps.local_execution);
        assert!(caps.structured_output);
    }

    #[test]
    fn opencode_id() {
        let adapter = OpenCodeAdapter::new(OpenCodeConfig::default());
        assert_eq!(adapter.id().as_str(), "opencode");
    }

    #[tokio::test]
    async fn opencode_submit_returns_unsupported() {
        use std::collections::HashMap;
        use std::path::PathBuf;

        use crate::adapters::traits::{ExecutionConstraints, ExecutionContext};
        use crate::models::{TaskId, TaskType};

        let adapter = OpenCodeAdapter::new(OpenCodeConfig::default());
        let request = ExecutionRequest {
            task_id: TaskId::new(),
            task_type: TaskType::Experimental,
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
