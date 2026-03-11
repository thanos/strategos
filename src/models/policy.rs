use serde::{Deserialize, Serialize};

use super::{ActionId, ProjectId, TaskId};

use chrono::{DateTime, Utc};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PendingActionType {
    CommitSuggestion,
    ReviewRequest,
    BudgetApproval,
    BackendOverride,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ActionStatus {
    Pending,
    Approved,
    Rejected,
    Expired,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingAction {
    pub id: ActionId,
    pub action_type: PendingActionType,
    pub project_id: ProjectId,
    pub task_id: Option<TaskId>,
    pub description: String,
    pub payload: serde_json::Value,
    pub status: ActionStatus,
    pub created_at: DateTime<Utc>,
}

impl PendingAction {
    pub fn new(
        action_type: PendingActionType,
        project_id: ProjectId,
        description: impl Into<String>,
    ) -> Self {
        Self {
            id: ActionId::new(),
            action_type,
            project_id,
            task_id: None,
            description: description.into(),
            payload: serde_json::Value::Null,
            status: ActionStatus::Pending,
            created_at: Utc::now(),
        }
    }

    pub fn with_task(mut self, task_id: TaskId) -> Self {
        self.task_id = Some(task_id);
        self
    }

    pub fn with_payload(mut self, payload: serde_json::Value) -> Self {
        self.payload = payload;
        self
    }
}
