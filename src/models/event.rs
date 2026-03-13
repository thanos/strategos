use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{EventId, ProjectId, TaskId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EventType {
    TaskSubmitted,
    TaskCompleted,
    TaskFailed,
    TaskCancelled,
    RoutingDecisionMade,
    BudgetThresholdReached,
    BudgetBlockTriggered,
    BackendDowngradeApplied,
    ReviewQueued,
    CommitSuggested,
    TaskQueued,
    ActionCreated,
    ActionApproved,
    ActionDismissed,
    WebhookDispatched,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub id: EventId,
    pub event_type: EventType,
    pub project_id: Option<ProjectId>,
    pub task_id: Option<TaskId>,
    pub payload: serde_json::Value,
    pub timestamp: DateTime<Utc>,
}

impl Event {
    pub fn new(event_type: EventType, payload: serde_json::Value) -> Self {
        Self {
            id: EventId::new(),
            event_type,
            project_id: None,
            task_id: None,
            payload,
            timestamp: Utc::now(),
        }
    }

    pub fn with_project(mut self, project_id: ProjectId) -> Self {
        self.project_id = Some(project_id);
        self
    }

    pub fn with_task(mut self, task_id: TaskId) -> Self {
        self.task_id = Some(task_id);
        self
    }
}

/// Record of a webhook delivery attempt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookDelivery {
    pub id: String,
    pub webhook_name: String,
    pub url: String,
    pub event_type: EventType,
    pub payload: serde_json::Value,
    pub status_code: Option<u16>,
    pub success: bool,
    pub error: Option<String>,
    pub delivered_at: DateTime<Utc>,
}
