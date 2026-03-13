use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{BackendId, Priority, ProjectId, TaskId, TaskType};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TaskStatus {
    Pending,
    Queued,
    Routed,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: TaskId,
    pub project_id: ProjectId,
    pub task_type: TaskType,
    pub description: String,
    pub priority: Priority,
    pub status: TaskStatus,
    pub backend_override: Option<BackendId>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub queued_at: Option<DateTime<Utc>>,
    pub tags: Vec<String>,
}

impl Task {
    pub fn new(
        project_id: ProjectId,
        task_type: TaskType,
        description: impl Into<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: TaskId::new(),
            project_id,
            task_type,
            description: description.into(),
            priority: Priority::default(),
            status: TaskStatus::Pending,
            backend_override: None,
            created_at: now,
            updated_at: now,
            queued_at: None,
            tags: Vec::new(),
        }
    }
}
