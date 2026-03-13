use std::sync::Arc;

use tracing::{info, warn};

use crate::adapters::traits::{AdapterRegistry, ExecutionContext, ExecutionRequest, ExecutionConstraints, ExecutionStatus};
use crate::budget::governor::BudgetGovernor;
use crate::errors::{AdapterError, RoutingError, StorageError};
#[allow(unused_imports)]
use crate::routing::engine::RoutingReason;
use crate::models::event::{Event, EventType};
use crate::models::policy::{ActionStatus, PendingAction};
use crate::models::project::Project;
use crate::models::task::{Task, TaskStatus};
use crate::models::usage::UsageRecord;
use crate::models::{ActionId, BackendId, MoneyAmount, ProjectId, TaskId};
use crate::storage::sqlite::RoutingHistoryRow;
use crate::routing::engine::{ProjectRoutingConfig, RoutingDecision, RoutingEngine, RoutingRequest};
use crate::storage::sqlite::SqliteStorage;

/// Configuration for automatic retry of transient adapter failures.
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    pub max_retries: u32,
    pub retry_delay: std::time::Duration,
    pub backoff_multiplier: f64,
    pub max_delay: std::time::Duration,
    pub jitter_fraction: f64,
}

impl RetryPolicy {
    /// Calculate the delay for a given attempt using exponential backoff with jitter.
    pub fn delay_for_attempt(&self, attempt: u32) -> std::time::Duration {
        if attempt == 0 {
            return std::time::Duration::ZERO;
        }
        let base_ms = self.retry_delay.as_millis() as f64;
        let exponential = base_ms * self.backoff_multiplier.powi((attempt - 1) as i32);
        let capped = exponential.min(self.max_delay.as_millis() as f64);

        // Apply jitter: delay * (1 - jitter_fraction * random)
        // Use a deterministic pseudo-random based on attempt for reproducibility in tests
        let jitter_range = capped * self.jitter_fraction;
        // Simple deterministic jitter: alternate between low and high
        let jitter = if attempt % 2 == 0 {
            jitter_range * 0.5
        } else {
            jitter_range
        };
        let final_ms = (capped - jitter).max(0.0);
        std::time::Duration::from_millis(final_ms as u64)
    }
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 0,
            retry_delay: std::time::Duration::from_millis(1000),
            backoff_multiplier: 2.0,
            max_delay: std::time::Duration::from_millis(30_000),
            jitter_fraction: 0.1,
        }
    }
}

/// The orchestrator wires together routing, budget, adapters, and storage.
/// It manages the full task lifecycle.
pub struct Orchestrator {
    pub registry: Arc<AdapterRegistry>,
    pub routing_engine: RoutingEngine,
    pub budget_governor: Arc<BudgetGovernor>,
    pub storage: Arc<SqliteStorage>,
    pub retry_policy: RetryPolicy,
    pub webhooks: Vec<crate::config::WebhookConfig>,
}

/// Result of submitting a task through the orchestrator.
pub struct SubmitResult {
    pub task: Task,
    pub routing_decision: RoutingDecision,
    pub execution_output: Option<String>,
    pub usage: Option<UsageRecord>,
    /// Set when the task requires budget approval before execution.
    pub requires_approval: bool,
    /// The pending action ID if approval was created.
    pub pending_action_id: Option<ActionId>,
}

/// Summary of budget state across all scopes.
pub struct BudgetSummary {
    pub global_spent: MoneyAmount,
    pub global_limit: MoneyAmount,
    pub backend_spend: Vec<(BackendId, MoneyAmount)>,
    pub project_spend: Vec<(ProjectId, String, MoneyAmount)>,
}

/// Per-project status for the overview dashboard.
pub struct ProjectStatusEntry {
    pub name: String,
    pub task_counts: Vec<(TaskStatus, usize)>,
    pub pending_actions: usize,
    pub month_spend: MoneyAmount,
}

impl Orchestrator {
    pub fn new(
        registry: Arc<AdapterRegistry>,
        routing_engine: RoutingEngine,
        budget_governor: Arc<BudgetGovernor>,
        storage: Arc<SqliteStorage>,
    ) -> Self {
        Self {
            registry,
            routing_engine,
            budget_governor,
            storage,
            retry_policy: RetryPolicy::default(),
            webhooks: Vec::new(),
        }
    }

    /// Set the retry policy for transient failures.
    pub fn with_retry_policy(mut self, policy: RetryPolicy) -> Self {
        self.retry_policy = policy;
        self
    }

    // -----------------------------------------------------------------------
    // Project management
    // -----------------------------------------------------------------------

    pub fn add_project(&self, project: &Project) -> Result<(), StorageError> {
        self.storage.insert_project(project)?;
        let event = Event::new(
            EventType::TaskSubmitted,
            serde_json::json!({"action": "project_added", "name": project.name}),
        )
        .with_project(project.id.clone());
        let _ = self.storage.insert_event(&event);
        info!(project = %project.name, "project added");
        Ok(())
    }

    pub fn list_projects(&self) -> Result<Vec<Project>, StorageError> {
        self.storage.list_projects()
    }

    pub fn get_project_by_name(&self, name: &str) -> Result<Option<Project>, StorageError> {
        self.storage.get_project_by_name(name)
    }

    pub fn remove_project(&self, id: &ProjectId) -> Result<(), StorageError> {
        self.storage.delete_project(id)?;
        info!(project_id = %id, "project removed");
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Task submission
    // -----------------------------------------------------------------------

    pub async fn submit_task(
        &self,
        task: Task,
        project_config: ProjectRoutingConfig,
        estimated_cost: MoneyAmount,
    ) -> Result<SubmitResult, SubmitError> {
        self.submit_task_with_context(task, project_config, estimated_cost, None, Vec::new(), ExecutionConstraints::default()).await
    }

    /// Submit a task with explicit execution context (project path, files) and constraints.
    pub async fn submit_task_with_context(
        &self,
        mut task: Task,
        project_config: ProjectRoutingConfig,
        estimated_cost: MoneyAmount,
        project_path: Option<std::path::PathBuf>,
        context_files: Vec<std::path::PathBuf>,
        constraints: ExecutionConstraints,
    ) -> Result<SubmitResult, SubmitError> {
        // Enforce max_cost_cents constraint before any work
        if let Some(max_cents) = constraints.max_cost_cents {
            if estimated_cost.cents > max_cents {
                return Err(SubmitError::Adapter(AdapterError::CostExceedsConstraint {
                    estimated_cents: estimated_cost.cents,
                    max_cents,
                }));
            }
        }
        // 1. Persist the task (skip if already stored, e.g. dequeued tasks)
        match self.storage.insert_task(&task) {
            Ok(()) => {}
            Err(StorageError::Database(ref msg)) if msg.contains("UNIQUE constraint") => {
                // Task already exists — that's fine (e.g. dequeued from queue)
            }
            Err(e) => return Err(SubmitError::Storage(e)),
        }

        // 2. Emit task submitted event
        let event = Event::new(
            EventType::TaskSubmitted,
            serde_json::json!({
                "task_type": format!("{:?}", task.task_type),
                "description": task.description,
            }),
        )
        .with_project(task.project_id.clone())
        .with_task(task.id.clone());
        let _ = self.storage.insert_event(&event);

        // 3. Route the task
        let routing_request = RoutingRequest {
            task_id: task.id.clone(),
            task_type: task.task_type,
            project_id: task.project_id.clone(),
            project_config,
            backend_override: task.backend_override.clone(),
            estimated_cost,
        };

        let decision = match self.routing_engine.route(routing_request).await {
            Ok(d) => d,
            Err(RoutingError::AllFallbacksFailed(ref evals))
                if evals.iter().any(|e| {
                    e.rejection_reason
                        .as_ref()
                        .map(|r| r.contains("budget approval") || r.contains("budget blocked"))
                        .unwrap_or(false)
                }) =>
            {
                let reason = evals
                    .iter()
                    .filter_map(|e| e.rejection_reason.as_ref())
                    .find(|r| r.contains("budget"))
                    .cloned()
                    .unwrap_or_else(|| "budget limit reached".into());

                // Budget requires approval — create a pending action
                let action = PendingAction::new(
                    crate::models::policy::PendingActionType::BudgetApproval,
                    task.project_id.clone(),
                    format!(
                        "Budget approval required for {:?} task: {}",
                        task.task_type, reason
                    ),
                )
                .with_task(task.id.clone())
                .with_payload(serde_json::json!({
                    "task_type": format!("{:?}", task.task_type),
                    "description": task.description,
                    "reason": reason,
                }));

                let _ = self.storage.insert_pending_action(&action);
                let _ = self.storage.insert_event(
                    &Event::new(
                        EventType::ActionCreated,
                        serde_json::json!({
                            "action_type": "BudgetApproval",
                            "reason": reason,
                        }),
                    )
                    .with_project(task.project_id.clone())
                    .with_task(task.id.clone()),
                );

                let _ = self.storage.update_task_status(&task.id, TaskStatus::Failed);

                return Ok(SubmitResult {
                    task,
                    routing_decision: RoutingDecision {
                        selected_backend: BackendId::new("none"),
                        reason: crate::routing::engine::RoutingReason::BudgetDowngrade {
                            original: BackendId::new("blocked"),
                        },
                        fallback_applied: false,
                        budget_downgrade_applied: false,
                        evaluated_backends: Vec::new(),
                    },
                    execution_output: None,
                    usage: None,
                    requires_approval: true,
                    pending_action_id: Some(action.id),
                });
            }
            Err(RoutingError::BudgetBlocked(reason)) => {
                // Budget requires approval — create a pending action
                let action = PendingAction::new(
                    crate::models::policy::PendingActionType::BudgetApproval,
                    task.project_id.clone(),
                    format!(
                        "Budget approval required for {:?} task: {}",
                        task.task_type, reason
                    ),
                )
                .with_task(task.id.clone())
                .with_payload(serde_json::json!({
                    "task_type": format!("{:?}", task.task_type),
                    "description": task.description,
                    "reason": reason,
                }));

                let _ = self.storage.insert_pending_action(&action);
                let _ = self.storage.insert_event(
                    &Event::new(
                        EventType::ActionCreated,
                        serde_json::json!({
                            "action_type": "BudgetApproval",
                            "reason": reason,
                        }),
                    )
                    .with_project(task.project_id.clone())
                    .with_task(task.id.clone()),
                );

                let _ = self.storage.update_task_status(&task.id, TaskStatus::Failed);

                return Ok(SubmitResult {
                    task,
                    routing_decision: RoutingDecision {
                        selected_backend: BackendId::new("none"),
                        reason: crate::routing::engine::RoutingReason::BudgetDowngrade {
                            original: BackendId::new("blocked"),
                        },
                        fallback_applied: false,
                        budget_downgrade_applied: false,
                        evaluated_backends: Vec::new(),
                    },
                    execution_output: None,
                    usage: None,
                    requires_approval: true,
                    pending_action_id: Some(action.id),
                });
            }
            Err(e) => return Err(SubmitError::Routing(e)),
        };

        info!(
            task_id = %task.id,
            backend = %decision.selected_backend,
            reason = ?decision.reason,
            "task routed"
        );

        // 4. Record routing history
        let _ = self.storage.insert_routing_history(
            &task.id,
            decision.selected_backend.as_str(),
            &format!("{:?}", decision.reason),
            decision.fallback_applied,
            decision.budget_downgrade_applied,
        );

        // 5. Emit routing decision event
        let routing_event = Event::new(
            EventType::RoutingDecisionMade,
            serde_json::json!({
                "backend": decision.selected_backend.as_str(),
                "reason": format!("{:?}", decision.reason),
                "fallback": decision.fallback_applied,
                "budget_downgrade": decision.budget_downgrade_applied,
            }),
        )
        .with_project(task.project_id.clone())
        .with_task(task.id.clone());
        let _ = self.storage.insert_event(&routing_event);

        // 6. Update task status to Routed
        task.status = TaskStatus::Routed;
        let _ = self.storage.update_task_status(&task.id, TaskStatus::Routed);

        // 7. Submit to adapter (with retry for transient failures)
        let adapter = self
            .registry
            .get(&decision.selected_backend)
            .ok_or_else(|| {
                SubmitError::Adapter(AdapterError::Unavailable(format!(
                    "backend {} not in registry",
                    decision.selected_backend
                )))
            })?;

        let build_exec_request = || ExecutionRequest {
            task_id: task.id.clone(),
            task_type: task.task_type,
            prompt: task.description.clone(),
            context: ExecutionContext {
                project_path: project_path.clone().unwrap_or_else(|| std::path::PathBuf::from(".")),
                working_directory: None,
                files: context_files.clone(),
                session_id: None,
                metadata: std::collections::HashMap::new(),
            },
            constraints: constraints.clone(),
        };

        let max_attempts = 1 + self.retry_policy.max_retries;
        let mut last_error: Option<AdapterError> = None;

        let (handle, status) = 'retry: {
            for attempt in 0..max_attempts {
                if attempt > 0 {
                    info!(task_id = %task.id, attempt, "retrying task after transient failure");
                    let _ = self.storage.insert_event(
                        &Event::new(
                            EventType::TaskSubmitted,
                            serde_json::json!({"action": "retry", "attempt": attempt}),
                        )
                        .with_project(task.project_id.clone())
                        .with_task(task.id.clone()),
                    );
                    tokio::time::sleep(self.retry_policy.delay_for_attempt(attempt)).await;
                }

                // Submit
                let handle = match adapter.submit(build_exec_request()).await {
                    Ok(h) => h,
                    Err(e) if e.is_transient() && attempt + 1 < max_attempts => {
                        warn!(error = %e, attempt, "transient submit failure, will retry");
                        last_error = Some(e);
                        continue;
                    }
                    Err(e) => {
                        warn!(error = %e, "adapter submit failed");
                        let _ = self.storage.update_task_status(&task.id, TaskStatus::Failed);
                        let fail_event = Event::new(
                            EventType::TaskFailed,
                            serde_json::json!({"error": e.to_string()}),
                        )
                        .with_project(task.project_id.clone())
                        .with_task(task.id.clone());
                        let _ = self.storage.insert_event(&fail_event);

                        return Ok(SubmitResult {
                            task,
                            routing_decision: decision,
                            execution_output: None,
                            usage: None,
                            requires_approval: false,
                            pending_action_id: None,
                        });
                    }
                };

                // 8. Update status to Running
                let _ = self.storage.update_task_status(&task.id, TaskStatus::Running);

                // 9. Poll for result (with optional timeout)
                let status = if let Some(timeout_duration) = constraints.timeout {
                    match tokio::time::timeout(timeout_duration, adapter.poll(&handle)).await {
                        Ok(result) => result,
                        Err(_) => {
                            warn!(task_id = %task.id, timeout = ?timeout_duration, "task timed out");
                            if attempt + 1 < max_attempts {
                                last_error = Some(AdapterError::Timeout(timeout_duration));
                                continue;
                            }
                            let _ = self.storage.update_task_status(&task.id, TaskStatus::Failed);
                            let timeout_event = Event::new(
                                EventType::TaskFailed,
                                serde_json::json!({"error": format!("timeout after {:?}", timeout_duration)}),
                            )
                            .with_project(task.project_id.clone())
                            .with_task(task.id.clone());
                            let _ = self.storage.insert_event(&timeout_event);
                            Ok(ExecutionStatus::Failed(AdapterError::Timeout(timeout_duration)))
                        }
                    }
                } else {
                    adapter.poll(&handle).await
                };

                // Check if poll result is a transient failure worth retrying
                if let Ok(ExecutionStatus::Failed(ref err)) = status {
                    if err.is_transient() && attempt + 1 < max_attempts {
                        warn!(error = %err, attempt, "transient poll failure, will retry");
                        last_error = Some(err.clone());
                        continue;
                    }
                }

                break 'retry (handle, status);
            }

            // All retries exhausted
            let err = last_error.unwrap_or_else(|| AdapterError::Internal("all retries exhausted".into()));
            let _ = self.storage.update_task_status(&task.id, TaskStatus::Failed);
            let fail_event = Event::new(
                EventType::TaskFailed,
                serde_json::json!({"error": err.to_string(), "retries_exhausted": true}),
            )
            .with_project(task.project_id.clone())
            .with_task(task.id.clone());
            let _ = self.storage.insert_event(&fail_event);

            return Ok(SubmitResult {
                task,
                routing_decision: decision,
                execution_output: None,
                usage: None,
                requires_approval: false,
                pending_action_id: None,
            });
        };

        let _ = handle; // handle consumed
        let (output, usage_record) = match status {
            Ok(ExecutionStatus::Completed(result)) => {
                let _ = self
                    .storage
                    .update_task_status(&task.id, TaskStatus::Completed);

                let usage = UsageRecord::new(
                    task.id.clone(),
                    task.project_id.clone(),
                    decision.selected_backend.clone(),
                    result.usage.input_tokens,
                    result.usage.output_tokens,
                    result.usage.cost,
                );
                let _ = self.storage.insert_usage(&usage);

                // Persist execution output
                let _ = self.storage.insert_task_output(
                    &task.id,
                    decision.selected_backend.as_str(),
                    &result.output,
                    result.structured_output.as_ref(),
                    result.usage.model.as_deref(),
                    result.usage.cost.cents,
                    result.usage.input_tokens,
                    result.usage.output_tokens,
                );

                let complete_event = Event::new(
                    EventType::TaskCompleted,
                    serde_json::json!({
                        "output_length": result.output.len(),
                        "cost_cents": result.usage.cost.cents,
                    }),
                )
                .with_project(task.project_id.clone())
                .with_task(task.id.clone());
                let _ = self.storage.insert_event(&complete_event);

                info!(
                    task_id = %task.id,
                    cost = %result.usage.cost,
                    "task completed"
                );

                (Some(result.output), Some(usage))
            }
            Ok(ExecutionStatus::Failed(e)) => {
                let _ = self.storage.update_task_status(&task.id, TaskStatus::Failed);
                warn!(task_id = %task.id, error = %e, "task execution failed");
                (None, None)
            }
            _ => {
                // Queued, Running, or Cancelled — return current state
                (None, None)
            }
        };

        Ok(SubmitResult {
            task,
            routing_decision: decision,
            execution_output: output,
            usage: usage_record,
            requires_approval: false,
            pending_action_id: None,
        })
    }

    // -----------------------------------------------------------------------
    // Task queue
    // -----------------------------------------------------------------------

    /// Queue a task for deferred execution. Sets status to Queued and records timestamp.
    pub fn queue_task(&self, task: &mut Task) -> Result<(), StorageError> {
        let now = chrono::Utc::now();
        task.status = TaskStatus::Queued;
        task.queued_at = Some(now);
        // Persist the task first if not already stored
        match self.storage.insert_task(task) {
            Ok(()) => {}
            Err(_) => {
                // Task already exists, just update queue status
            }
        }
        self.storage.queue_task(&task.id)?;

        let event = Event::new(
            EventType::TaskQueued,
            serde_json::json!({
                "task_type": format!("{:?}", task.task_type),
                "priority": format!("{:?}", task.priority),
                "rank": task.priority.rank(),
            }),
        )
        .with_project(task.project_id.clone())
        .with_task(task.id.clone());
        let _ = self.record_and_dispatch_event(event);

        info!(task_id = %task.id, priority = ?task.priority, "task queued");
        Ok(())
    }

    /// Dequeue and execute the highest-priority queued task.
    pub async fn run_next_queued(
        &self,
        project_config: ProjectRoutingConfig,
        estimated_cost: MoneyAmount,
    ) -> Result<Option<SubmitResult>, SubmitError> {
        let task = self
            .storage
            .dequeue_next_task()
            .map_err(SubmitError::Storage)?;

        match task {
            Some(task) => {
                let result = self
                    .submit_task(task, project_config, estimated_cost)
                    .await?;
                Ok(Some(result))
            }
            None => Ok(None),
        }
    }

    /// List all queued tasks ordered by priority.
    pub fn list_queued_tasks(&self) -> Result<Vec<Task>, StorageError> {
        self.storage.list_queued_tasks()
    }

    /// Count queued tasks.
    pub fn count_queued_tasks(&self) -> Result<usize, StorageError> {
        self.storage.count_queued_tasks()
    }

    // -----------------------------------------------------------------------
    // Webhook dispatch
    // -----------------------------------------------------------------------

    /// Record an event and dispatch it to configured webhooks.
    pub fn record_and_dispatch_event(&self, event: Event) -> Result<(), StorageError> {
        self.storage.insert_event(&event)?;
        self.dispatch_webhooks(&event);
        Ok(())
    }

    /// Dispatch an event to all matching webhooks.
    pub fn dispatch_webhooks(&self, event: &Event) {
        let event_type_str = format!("{:?}", event.event_type);
        for webhook in &self.webhooks {
            if !webhook.enabled {
                continue;
            }
            // Check event filter
            if let Some(ref filter) = webhook.events {
                if !filter.is_empty() && !filter.iter().any(|e| e == &event_type_str) {
                    continue;
                }
            }
            // Record the delivery (simulated — actual HTTP would be async)
            let delivery = crate::models::event::WebhookDelivery {
                id: uuid::Uuid::new_v4().to_string(),
                webhook_name: webhook.name.clone(),
                url: webhook.url.clone(),
                event_type: event.event_type,
                payload: event.payload.clone(),
                status_code: Some(200),
                success: true,
                error: None,
                delivered_at: chrono::Utc::now(),
            };
            let _ = self.storage.insert_webhook_delivery(&delivery);
        }
    }

    // -----------------------------------------------------------------------
    // Budget status
    // -----------------------------------------------------------------------

    pub fn budget_summary(
        &self,
        global_limit: MoneyAmount,
        year_month: &str,
    ) -> Result<BudgetSummary, StorageError> {
        let global_spent = self.storage.total_spend_month(year_month)?;

        // Collect spend per backend
        let mut backend_spend = Vec::new();
        for backend_id in self.registry.list() {
            let spent = self.storage.backend_spend_month(backend_id, year_month)?;
            if spent.cents > 0 {
                backend_spend.push((backend_id.clone(), spent));
            }
        }

        // Collect spend per project
        let mut project_spend = Vec::new();
        for project in self.storage.list_projects()? {
            let spent = self.storage.project_spend_month(&project.id, year_month)?;
            if spent.cents > 0 {
                project_spend.push((project.id.clone(), project.name.clone(), spent));
            }
        }

        Ok(BudgetSummary {
            global_spent,
            global_limit,
            backend_spend,
            project_spend,
        })
    }

    // -----------------------------------------------------------------------
    // Events
    // -----------------------------------------------------------------------

    pub fn recent_events(&self, limit: usize) -> Result<Vec<Event>, StorageError> {
        self.storage.list_events_recent(limit)
    }

    pub fn filtered_events(
        &self,
        event_type: Option<&str>,
        project_id: Option<&ProjectId>,
        task_id: Option<&TaskId>,
        since: Option<&str>,
        until: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Event>, StorageError> {
        self.storage.list_events_filtered(event_type, project_id, task_id, since, until, limit)
    }

    // -----------------------------------------------------------------------
    // Tasks
    // -----------------------------------------------------------------------

    pub fn list_tasks(&self, project_id: &ProjectId) -> Result<Vec<Task>, StorageError> {
        self.storage.list_tasks_by_project(project_id)
    }

    pub fn get_task(&self, id: &TaskId) -> Result<Option<Task>, StorageError> {
        self.storage.get_task(id)
    }

    /// Cancel a task. Only Pending, Routed, or Running tasks can be cancelled.
    pub fn cancel_task(&self, id: &TaskId) -> Result<(), CancelError> {
        let task = self
            .storage
            .get_task(id)
            .map_err(CancelError::Storage)?
            .ok_or_else(|| CancelError::Storage(StorageError::NotFound(format!("task {}", id.0))))?;

        match task.status {
            TaskStatus::Pending | TaskStatus::Queued | TaskStatus::Routed | TaskStatus::Running => {}
            other => {
                return Err(CancelError::InvalidState(format!(
                    "cannot cancel task in {:?} state",
                    other
                )));
            }
        }

        self.storage
            .update_task_status(id, TaskStatus::Cancelled)
            .map_err(CancelError::Storage)?;

        let event = Event::new(
            EventType::TaskCancelled,
            serde_json::json!({
                "task_type": format!("{:?}", task.task_type),
                "previous_status": format!("{:?}", task.status),
            }),
        )
        .with_project(task.project_id.clone())
        .with_task(id.clone());
        let _ = self.storage.insert_event(&event);

        info!(task_id = %id.0, "task cancelled");
        Ok(())
    }

    /// Register task dependencies in storage.
    pub fn add_task_dependencies(
        &self,
        task_id: &TaskId,
        depends_on: &[TaskId],
    ) -> Result<(), StorageError> {
        for dep_id in depends_on {
            self.storage.insert_task_dependency(task_id, dep_id)?;
        }
        Ok(())
    }

    /// Check if all dependencies for a task are completed.
    pub fn check_dependencies(&self, task_id: &TaskId) -> Result<bool, StorageError> {
        self.storage.all_dependencies_completed(task_id)
    }

    /// Get the list of dependency task IDs for a task.
    pub fn get_task_dependencies(&self, task_id: &TaskId) -> Result<Vec<TaskId>, StorageError> {
        self.storage.get_task_dependencies(task_id)
    }

    pub fn get_routing_history_for_task(
        &self,
        task_id: &TaskId,
    ) -> Result<Option<RoutingHistoryRow>, StorageError> {
        self.storage.get_routing_history_for_task(task_id)
    }

    // -----------------------------------------------------------------------
    // Pending actions
    // -----------------------------------------------------------------------

    pub fn list_pending_actions(&self) -> Result<Vec<PendingAction>, StorageError> {
        self.storage.list_pending_actions()
    }

    pub fn list_all_actions(&self, limit: usize) -> Result<Vec<PendingAction>, StorageError> {
        self.storage.list_all_actions(limit)
    }

    pub fn get_pending_action(
        &self,
        id: &ActionId,
    ) -> Result<Option<PendingAction>, StorageError> {
        self.storage.get_pending_action(id)
    }

    pub fn approve_action(&self, id: &ActionId) -> Result<(), StorageError> {
        let action = self
            .storage
            .get_pending_action(id)?
            .ok_or_else(|| StorageError::NotFound(format!("action {}", id.0)))?;

        self.storage.update_action_status(id, ActionStatus::Approved)?;

        let event = Event::new(
            EventType::ActionApproved,
            serde_json::json!({
                "action_type": format!("{:?}", action.action_type),
                "description": action.description,
            }),
        )
        .with_project(action.project_id.clone());
        let _ = self.storage.insert_event(&event);

        info!(action_id = %id.0, "action approved");
        Ok(())
    }

    pub fn dismiss_action(&self, id: &ActionId) -> Result<(), StorageError> {
        let action = self
            .storage
            .get_pending_action(id)?
            .ok_or_else(|| StorageError::NotFound(format!("action {}", id.0)))?;

        self.storage.update_action_status(id, ActionStatus::Rejected)?;

        let event = Event::new(
            EventType::ActionDismissed,
            serde_json::json!({
                "action_type": format!("{:?}", action.action_type),
                "description": action.description,
            }),
        )
        .with_project(action.project_id.clone());
        let _ = self.storage.insert_event(&event);

        info!(action_id = %id.0, "action dismissed");
        Ok(())
    }

    pub fn create_action(&self, action: &PendingAction) -> Result<(), StorageError> {
        self.storage.insert_pending_action(action)?;

        let event = Event::new(
            EventType::ActionCreated,
            serde_json::json!({
                "action_type": format!("{:?}", action.action_type),
                "description": action.description,
            }),
        )
        .with_project(action.project_id.clone());
        if let Some(ref task_id) = action.task_id {
            let event = event.with_task(task_id.clone());
            let _ = self.storage.insert_event(&event);
        } else {
            let _ = self.storage.insert_event(&event);
        }

        info!(action_id = %action.id.0, "action created");
        Ok(())
    }

    pub fn list_actions_for_task(
        &self,
        task_id: &TaskId,
    ) -> Result<Vec<PendingAction>, StorageError> {
        self.storage.list_actions_for_task(task_id)
    }

    // -----------------------------------------------------------------------
    // Status overview
    // -----------------------------------------------------------------------

    pub fn project_status_summary(&self) -> Result<Vec<ProjectStatusEntry>, StorageError> {
        let projects = self.storage.list_projects()?;
        let year_month = chrono::Utc::now().format("%Y-%m").to_string();

        let mut entries = Vec::new();
        for project in projects {
            let task_counts = self.storage.count_tasks_by_status(&project.id)?;
            let pending_actions = self.storage.count_pending_actions_for_project(&project.id)?;
            let spend = self.storage.project_spend_month(&project.id, &year_month)?;

            entries.push(ProjectStatusEntry {
                name: project.name,
                task_counts,
                pending_actions,
                month_spend: spend,
            });
        }
        Ok(entries)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SubmitError {
    #[error("storage error: {0}")]
    Storage(StorageError),
    #[error("routing error: {0}")]
    Routing(RoutingError),
    #[error("adapter error: {0}")]
    Adapter(AdapterError),
    #[error("unsatisfied dependencies: {0}")]
    UnsatisfiedDependencies(String),
}

#[derive(Debug, thiserror::Error)]
pub enum CancelError {
    #[error("storage error: {0}")]
    Storage(StorageError),
    #[error("invalid state: {0}")]
    InvalidState(String),
}
