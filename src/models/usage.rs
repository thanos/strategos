use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{BackendId, MoneyAmount, ProjectId, TaskId, UsageId};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageRecord {
    pub id: UsageId,
    pub task_id: TaskId,
    pub project_id: ProjectId,
    pub backend_id: BackendId,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost: MoneyAmount,
    pub model: Option<String>,
    pub recorded_at: DateTime<Utc>,
}

impl UsageRecord {
    pub fn new(
        task_id: TaskId,
        project_id: ProjectId,
        backend_id: BackendId,
        input_tokens: u64,
        output_tokens: u64,
        cost: MoneyAmount,
    ) -> Self {
        Self {
            id: UsageId::new(),
            task_id,
            project_id,
            backend_id,
            input_tokens,
            output_tokens,
            cost,
            model: None,
            recorded_at: Utc::now(),
        }
    }

    pub fn total_tokens(&self) -> u64 {
        self.input_tokens + self.output_tokens
    }
}
