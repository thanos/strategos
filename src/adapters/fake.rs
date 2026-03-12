use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use uuid::Uuid;

use crate::errors::AdapterError;
use crate::models::{BackendId, MoneyAmount};

use super::traits::{
    AdapterCapabilities, ExecutionAdapter, ExecutionHandle, ExecutionRequest, ExecutionResult,
    ExecutionStatus, HealthStatus, UsageReport,
};

/// Configurable test double for the ExecutionAdapter trait.
pub struct FakeAdapter {
    backend_id: BackendId,
    capabilities: AdapterCapabilities,
    behavior: FakeBehavior,
}

/// Controls how the FakeAdapter responds to submissions.
pub enum FakeBehavior {
    /// Immediately returns a successful result.
    SucceedImmediately { output: String },
    /// Immediately returns an error.
    FailWith(AdapterError),
    /// Returns success with custom usage.
    SucceedWithUsage {
        output: String,
        input_tokens: u64,
        output_tokens: u64,
        cost: MoneyAmount,
    },
}

impl FakeAdapter {
    pub fn new(
        name: impl Into<String>,
        capabilities: AdapterCapabilities,
        behavior: FakeBehavior,
    ) -> Self {
        Self {
            backend_id: BackendId::new(name),
            capabilities,
            behavior,
        }
    }

    /// Creates a FakeAdapter that succeeds immediately with default capabilities.
    pub fn succeeding(name: impl Into<String>) -> Self {
        Self {
            backend_id: BackendId::new(name),
            capabilities: Self::full_capabilities(),
            behavior: FakeBehavior::SucceedImmediately {
                output: "fake success".into(),
            },
        }
    }

    /// Creates a FakeAdapter that always fails.
    pub fn failing(name: impl Into<String>, error: AdapterError) -> Self {
        Self {
            backend_id: BackendId::new(name),
            capabilities: Self::full_capabilities(),
            behavior: FakeBehavior::FailWith(error),
        }
    }

    /// Creates a FakeAdapter with local-only capabilities.
    pub fn local(name: impl Into<String>) -> Self {
        Self {
            backend_id: BackendId::new(name),
            capabilities: AdapterCapabilities {
                code_editing: false,
                shell_tool_use: false,
                multi_step_agent: false,
                local_execution: true,
                structured_output: false,
                streaming: false,
                subagents: false,
                session_resume: false,
            },
            behavior: FakeBehavior::SucceedImmediately {
                output: "local fake success".into(),
            },
        }
    }

    pub fn full_capabilities() -> AdapterCapabilities {
        AdapterCapabilities {
            code_editing: true,
            shell_tool_use: true,
            multi_step_agent: true,
            local_execution: false,
            structured_output: true,
            streaming: true,
            subagents: true,
            session_resume: true,
        }
    }

    fn make_handle(&self) -> ExecutionHandle {
        ExecutionHandle {
            backend_id: self.backend_id.clone(),
            handle_id: Uuid::new_v4().to_string(),
            submitted_at: Utc::now(),
        }
    }

    fn make_result(&self, output: &str, usage: UsageReport) -> ExecutionResult {
        ExecutionResult {
            output: output.to_string(),
            structured_output: None,
            files_modified: Vec::new(),
            usage,
            completed_at: Utc::now(),
        }
    }
}

#[async_trait]
impl ExecutionAdapter for FakeAdapter {
    fn id(&self) -> &BackendId {
        &self.backend_id
    }

    fn capabilities(&self) -> &AdapterCapabilities {
        &self.capabilities
    }

    async fn health_check(&self) -> HealthStatus {
        match &self.behavior {
            FakeBehavior::FailWith(AdapterError::Unavailable(msg)) => {
                HealthStatus::Unavailable(msg.clone())
            }
            _ => HealthStatus::Healthy,
        }
    }

    async fn submit(&self, _request: ExecutionRequest) -> Result<ExecutionHandle, AdapterError> {
        match &self.behavior {
            FakeBehavior::FailWith(e) => Err(match e {
                AdapterError::Unavailable(msg) => AdapterError::Unavailable(msg.clone()),
                AdapterError::AuthenticationFailed(msg) => {
                    AdapterError::AuthenticationFailed(msg.clone())
                }
                AdapterError::RequestFailed(msg) => AdapterError::RequestFailed(msg.clone()),
                AdapterError::Unsupported(msg) => AdapterError::Unsupported(msg.clone()),
                AdapterError::Internal(msg) => AdapterError::Internal(msg.clone()),
                _ => AdapterError::Internal("fake error".into()),
            }),
            _ => Ok(self.make_handle()),
        }
    }

    async fn poll(&self, _handle: &ExecutionHandle) -> Result<ExecutionStatus, AdapterError> {
        match &self.behavior {
            FakeBehavior::SucceedImmediately { output } => {
                let result = self.make_result(output, UsageReport::zero());
                Ok(ExecutionStatus::Completed(result))
            }
            FakeBehavior::SucceedWithUsage {
                output,
                input_tokens,
                output_tokens,
                cost,
            } => {
                let usage = UsageReport {
                    input_tokens: *input_tokens,
                    output_tokens: *output_tokens,
                    total_tokens: input_tokens + output_tokens,
                    cost: *cost,
                    model: Some("fake-model".into()),
                    duration: Duration::from_millis(100),
                };
                let result = self.make_result(output, usage);
                Ok(ExecutionStatus::Completed(result))
            }
            FakeBehavior::FailWith(e) => Ok(ExecutionStatus::Failed(match e {
                AdapterError::Unavailable(msg) => AdapterError::Unavailable(msg.clone()),
                _ => AdapterError::Internal("fake error".into()),
            })),
        }
    }

    async fn cancel(&self, _handle: &ExecutionHandle) -> Result<(), AdapterError> {
        Ok(())
    }

    async fn usage(&self, _handle: &ExecutionHandle) -> Result<UsageReport, AdapterError> {
        match &self.behavior {
            FakeBehavior::SucceedWithUsage {
                input_tokens,
                output_tokens,
                cost,
                ..
            } => Ok(UsageReport {
                input_tokens: *input_tokens,
                output_tokens: *output_tokens,
                total_tokens: input_tokens + output_tokens,
                cost: *cost,
                model: Some("fake-model".into()),
                duration: Duration::from_millis(100),
            }),
            _ => Ok(UsageReport::zero()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use crate::models::{TaskId, TaskType};

    use super::*;

    fn test_request() -> ExecutionRequest {
        ExecutionRequest {
            task_id: TaskId::new(),
            task_type: TaskType::Summarization,
            prompt: "test prompt".into(),
            context: super::super::traits::ExecutionContext {
                project_path: PathBuf::from("/tmp/test"),
                working_directory: None,
                files: Vec::new(),
                session_id: None,
                metadata: HashMap::new(),
            },
            constraints: super::super::traits::ExecutionConstraints::default(),
        }
    }

    #[tokio::test]
    async fn fake_succeed_immediately() {
        let adapter = FakeAdapter::succeeding("test");
        let handle = adapter.submit(test_request()).await.unwrap();
        let status = adapter.poll(&handle).await.unwrap();
        match status {
            ExecutionStatus::Completed(result) => {
                assert_eq!(result.output, "fake success");
            }
            _ => panic!("expected Completed"),
        }
    }

    #[tokio::test]
    async fn fake_fail_on_submit() {
        let adapter = FakeAdapter::failing("test", AdapterError::Unavailable("down".into()));
        let result = adapter.submit(test_request()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn fake_with_usage() {
        let adapter = FakeAdapter::new(
            "test",
            FakeAdapter::full_capabilities(),
            FakeBehavior::SucceedWithUsage {
                output: "done".into(),
                input_tokens: 100,
                output_tokens: 50,
                cost: MoneyAmount::from_cents(25),
            },
        );
        let handle = adapter.submit(test_request()).await.unwrap();
        let usage = adapter.usage(&handle).await.unwrap();
        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.output_tokens, 50);
        assert_eq!(usage.cost.cents, 25);
    }

    #[tokio::test]
    async fn fake_cancel_always_ok() {
        let adapter = FakeAdapter::succeeding("test");
        let handle = adapter.submit(test_request()).await.unwrap();
        assert!(adapter.cancel(&handle).await.is_ok());
    }

    #[tokio::test]
    async fn fake_local_capabilities() {
        let adapter = FakeAdapter::local("local-test");
        assert!(adapter.capabilities().local_execution);
        assert!(!adapter.capabilities().code_editing);
    }

    #[tokio::test]
    async fn fake_health_check_healthy() {
        let adapter = FakeAdapter::succeeding("test");
        let status = adapter.health_check().await;
        assert!(matches!(status, HealthStatus::Healthy));
    }

    #[tokio::test]
    async fn fake_health_check_unavailable() {
        let adapter = FakeAdapter::failing("test", AdapterError::Unavailable("server down".into()));
        let status = adapter.health_check().await;
        assert!(matches!(status, HealthStatus::Unavailable(_)));
    }
}
