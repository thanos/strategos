use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::errors::AdapterError;
use crate::models::{BackendId, MoneyAmount, TaskId, TaskType};

/// The provider-neutral execution adapter contract.
/// Every backend — Claude, Ollama, OpenCode — implements this trait.
#[async_trait]
pub trait ExecutionAdapter: Send + Sync {
    /// Unique identifier for this backend.
    fn id(&self) -> &BackendId;

    /// Reports the capabilities of this backend.
    fn capabilities(&self) -> &AdapterCapabilities;

    /// Submit a task for execution.
    async fn submit(&self, request: ExecutionRequest) -> Result<ExecutionHandle, AdapterError>;

    /// Poll the status of a submitted task.
    async fn poll(&self, handle: &ExecutionHandle) -> Result<ExecutionStatus, AdapterError>;

    /// Cancel a running task.
    async fn cancel(&self, handle: &ExecutionHandle) -> Result<(), AdapterError>;

    /// Retrieve usage information for a task.
    async fn usage(&self, handle: &ExecutionHandle) -> Result<UsageReport, AdapterError>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionRequest {
    pub task_id: TaskId,
    pub task_type: TaskType,
    pub prompt: String,
    pub context: ExecutionContext,
    pub constraints: ExecutionConstraints,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionContext {
    pub project_path: PathBuf,
    pub working_directory: Option<PathBuf>,
    pub files: Vec<PathBuf>,
    pub session_id: Option<String>,
    pub metadata: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionConstraints {
    pub max_tokens: Option<u64>,
    pub max_cost_cents: Option<i64>,
    pub timeout: Option<Duration>,
    pub allow_shell: bool,
    pub allow_file_edit: bool,
}

impl Default for ExecutionConstraints {
    fn default() -> Self {
        Self {
            max_tokens: None,
            max_cost_cents: None,
            timeout: None,
            allow_shell: false,
            allow_file_edit: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionHandle {
    pub backend_id: BackendId,
    pub handle_id: String,
    pub submitted_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub enum ExecutionStatus {
    Queued,
    Running { progress: Option<String> },
    Completed(ExecutionResult),
    Failed(AdapterError),
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionResult {
    pub output: String,
    pub structured_output: Option<serde_json::Value>,
    pub files_modified: Vec<PathBuf>,
    pub usage: UsageReport,
    pub completed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageReport {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub cost: MoneyAmount,
    pub model: Option<String>,
    pub duration: Duration,
}

impl UsageReport {
    pub fn zero() -> Self {
        Self {
            input_tokens: 0,
            output_tokens: 0,
            total_tokens: 0,
            cost: MoneyAmount::ZERO,
            model: None,
            duration: Duration::ZERO,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdapterCapabilities {
    pub code_editing: bool,
    pub shell_tool_use: bool,
    pub multi_step_agent: bool,
    pub local_execution: bool,
    pub structured_output: bool,
    pub streaming: bool,
    pub subagents: bool,
    pub session_resume: bool,
}

impl AdapterCapabilities {
    /// Returns true if this adapter satisfies the requirements for the given task type.
    pub fn supports_task_type(&self, task_type: &TaskType) -> bool {
        match task_type {
            TaskType::DeepCodeReasoning => self.code_editing && self.multi_step_agent,
            TaskType::Planning => self.structured_output,
            TaskType::Review => self.code_editing,
            TaskType::CommitPreparation => self.code_editing && self.shell_tool_use,
            TaskType::PrivateLocalTask => self.local_execution,
            // These task types have no special capability requirements.
            TaskType::Summarization
            | TaskType::BacklogTriage
            | TaskType::LowCostDrafting
            | TaskType::Experimental => true,
        }
    }
}

/// Registry of available execution adapters.
pub struct AdapterRegistry {
    adapters: HashMap<BackendId, Arc<dyn ExecutionAdapter>>,
}

impl AdapterRegistry {
    pub fn new() -> Self {
        Self {
            adapters: HashMap::new(),
        }
    }

    pub fn register(&mut self, adapter: Arc<dyn ExecutionAdapter>) {
        let id = adapter.id().clone();
        self.adapters.insert(id, adapter);
    }

    pub fn get(&self, id: &BackendId) -> Option<&Arc<dyn ExecutionAdapter>> {
        self.adapters.get(id)
    }

    pub fn list(&self) -> Vec<&BackendId> {
        self.adapters.keys().collect()
    }

    pub fn is_available(&self, id: &BackendId) -> bool {
        self.adapters.contains_key(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capabilities_deep_code_reasoning_requires_editing_and_agent() {
        let caps = AdapterCapabilities {
            code_editing: true,
            shell_tool_use: false,
            multi_step_agent: true,
            local_execution: false,
            structured_output: false,
            streaming: false,
            subagents: false,
            session_resume: false,
        };
        assert!(caps.supports_task_type(&TaskType::DeepCodeReasoning));

        let no_agent = AdapterCapabilities {
            multi_step_agent: false,
            ..caps.clone()
        };
        assert!(!no_agent.supports_task_type(&TaskType::DeepCodeReasoning));
    }

    #[test]
    fn capabilities_private_local_requires_local_execution() {
        let caps = AdapterCapabilities {
            code_editing: false,
            shell_tool_use: false,
            multi_step_agent: false,
            local_execution: true,
            structured_output: false,
            streaming: false,
            subagents: false,
            session_resume: false,
        };
        assert!(caps.supports_task_type(&TaskType::PrivateLocalTask));

        let no_local = AdapterCapabilities {
            local_execution: false,
            ..caps
        };
        assert!(!no_local.supports_task_type(&TaskType::PrivateLocalTask));
    }

    #[test]
    fn capabilities_summarization_has_no_requirements() {
        let minimal = AdapterCapabilities {
            code_editing: false,
            shell_tool_use: false,
            multi_step_agent: false,
            local_execution: false,
            structured_output: false,
            streaming: false,
            subagents: false,
            session_resume: false,
        };
        assert!(minimal.supports_task_type(&TaskType::Summarization));
        assert!(minimal.supports_task_type(&TaskType::BacklogTriage));
        assert!(minimal.supports_task_type(&TaskType::LowCostDrafting));
        assert!(minimal.supports_task_type(&TaskType::Experimental));
    }

    #[test]
    fn registry_register_and_get() {
        use crate::adapters::fake::FakeAdapter;

        let mut registry = AdapterRegistry::new();
        let adapter = Arc::new(FakeAdapter::succeeding("test-backend"));
        registry.register(adapter);

        let id = BackendId::new("test-backend");
        assert!(registry.get(&id).is_some());
        assert!(registry.is_available(&id));
        assert_eq!(registry.list().len(), 1);
    }

    #[test]
    fn registry_get_unknown_returns_none() {
        let registry = AdapterRegistry::new();
        assert!(registry.get(&BackendId::new("nonexistent")).is_none());
    }

    #[test]
    fn usage_report_zero() {
        let r = UsageReport::zero();
        assert_eq!(r.input_tokens, 0);
        assert_eq!(r.cost, MoneyAmount::ZERO);
    }
}
