use thiserror::Error;

use crate::models::BackendId;

#[derive(Debug, Error)]
pub enum StrategosError {
    #[error("configuration error: {0}")]
    Config(String),

    #[error("storage error: {0}")]
    Storage(#[from] StorageError),

    #[error("adapter error: {0}")]
    Adapter(#[from] AdapterError),

    #[error("routing error: {0}")]
    Routing(#[from] RoutingError),

    #[error("budget error: {0}")]
    Budget(#[from] BudgetError),
}

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("not found: {0}")]
    NotFound(String),

    #[error("constraint violation: {0}")]
    ConstraintViolation(String),

    #[error("database error: {0}")]
    Database(String),

    #[error("serialization error: {0}")]
    Serialization(String),
}

#[derive(Debug, Clone, Error)]
pub enum AdapterError {
    #[error("authentication failed: {0}")]
    AuthenticationFailed(String),

    #[error("rate limited, retry after {retry_after:?}")]
    RateLimited {
        retry_after: Option<std::time::Duration>,
    },

    #[error("backend unavailable: {0}")]
    Unavailable(String),

    #[error("request failed: {0}")]
    RequestFailed(String),

    #[error("timeout after {0:?}")]
    Timeout(std::time::Duration),

    #[error("task not found: {0}")]
    TaskNotFound(String),

    #[error("unsupported operation: {0}")]
    Unsupported(String),

    #[error("internal error: {0}")]
    Internal(String),

    #[error("cost exceeds constraint: estimated {estimated_cents} cents, max {max_cents} cents")]
    CostExceedsConstraint {
        estimated_cents: i64,
        max_cents: i64,
    },
}

impl AdapterError {
    /// Returns true if this error is transient and the operation may succeed on retry.
    pub fn is_transient(&self) -> bool {
        matches!(
            self,
            AdapterError::RateLimited { .. }
                | AdapterError::Unavailable(_)
                | AdapterError::Timeout(_)
                | AdapterError::RequestFailed(_)
        )
    }
}

#[derive(Debug, Error)]
pub enum RoutingError {
    #[error("no eligible backend for task type {task_type}: {reason}")]
    NoEligibleBackend { task_type: String, reason: String },

    #[error("budget blocked: {0}")]
    BudgetBlocked(String),

    #[error("all fallbacks failed")]
    AllFallbacksFailed(Vec<BackendEvaluation>),
}

#[derive(Debug, Clone)]
pub struct BackendEvaluation {
    pub backend_id: BackendId,
    pub eligible: bool,
    pub rejection_reason: Option<String>,
}

#[derive(Debug, Error)]
pub enum BudgetError {
    #[error("storage error: {0}")]
    Storage(String),

    #[error("invalid configuration: {0}")]
    InvalidConfig(String),
}
