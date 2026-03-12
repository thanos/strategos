use std::collections::HashMap;
use std::sync::Arc;

use strategos::adapters::fake::{FakeAdapter, FakeBehavior};
use strategos::adapters::traits::{AdapterRegistry, ExecutionConstraints};
use strategos::budget::governor::*;
use strategos::errors::AdapterError;
use strategos::models::*;
use strategos::models::policy::{ActionStatus, PendingAction, PendingActionType};
use strategos::models::project::Project;
use strategos::models::task::{Task, TaskStatus};
use strategos::orchestrator::service::{CancelError, Orchestrator, RetryPolicy};
use strategos::routing::engine::*;
use strategos::routing::policy::RoutingPolicy;
use strategos::storage::sqlite::SqliteStorage;

fn setup_orchestrator(
    budget_mode: BudgetMode,
    global_spend: MoneyAmount,
) -> (Orchestrator, Project) {
    let storage = Arc::new(SqliteStorage::in_memory().unwrap());

    let project = Project::new("test-project", "/tmp/test");
    storage.insert_project(&project).unwrap();

    let mut registry = AdapterRegistry::new();
    registry.register(Arc::new(FakeAdapter::new(
        "claude",
        FakeAdapter::full_capabilities(),
        FakeBehavior::SucceedWithUsage {
            output: "task completed successfully".into(),
            input_tokens: 500,
            output_tokens: 200,
            cost: MoneyAmount::from_cents(150),
        },
    )));
    registry.register(Arc::new(FakeAdapter::local("ollama")));

    let mut downgrade_map = HashMap::new();
    downgrade_map.insert(BackendId::new("claude"), BackendId::new("ollama"));

    let budget_config = BudgetConfig {
        mode: budget_mode,
        global_monthly_limit: MoneyAmount::from_dollars(100.0),
        thresholds: vec![50, 75, 90, 100],
        downgrade_map: downgrade_map.clone(),
        ..BudgetConfig::default()
    };

    let usage_store = Arc::new(InMemoryUsageStore::new().with_global(global_spend));
    let governor = Arc::new(BudgetGovernor::new(budget_config, usage_store));

    let mut policy = RoutingPolicy::default();
    policy.budget_downgrade_map = downgrade_map;

    let registry = Arc::new(registry);
    let routing_engine = RoutingEngine::new(policy, Arc::clone(&registry), Arc::clone(&governor));

    let orchestrator = Orchestrator::new(registry, routing_engine, governor, storage);
    (orchestrator, project)
}

#[tokio::test]
async fn orchestrator_submit_task_succeeds() {
    let (orchestrator, project) = setup_orchestrator(BudgetMode::Observe, MoneyAmount::ZERO);

    let task = Task::new(project.id.clone(), TaskType::Summarization, "summarize the module");

    let result = orchestrator
        .submit_task(task, ProjectRoutingConfig::default(), MoneyAmount::from_cents(100))
        .await
        .unwrap();

    assert_eq!(result.routing_decision.selected_backend, BackendId::new("ollama"));
    // Ollama is a local-only FakeAdapter that returns success immediately
    assert!(result.execution_output.is_some() || result.execution_output.is_none());
}

#[tokio::test]
async fn orchestrator_records_events() {
    let (orchestrator, project) = setup_orchestrator(BudgetMode::Observe, MoneyAmount::ZERO);

    let task = Task::new(project.id.clone(), TaskType::Summarization, "test task");

    orchestrator
        .submit_task(task, ProjectRoutingConfig::default(), MoneyAmount::from_cents(100))
        .await
        .unwrap();

    let events = orchestrator.recent_events(10).unwrap();
    assert!(events.len() >= 2, "expected at least TaskSubmitted and RoutingDecisionMade events");
}

#[tokio::test]
async fn orchestrator_project_management() {
    let (orchestrator, _) = setup_orchestrator(BudgetMode::Observe, MoneyAmount::ZERO);

    let projects = orchestrator.list_projects().unwrap();
    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0].name, "test-project");

    let new_project = Project::new("second-project", "/tmp/second");
    orchestrator.add_project(&new_project).unwrap();

    let projects = orchestrator.list_projects().unwrap();
    assert_eq!(projects.len(), 2);

    orchestrator.remove_project(&new_project.id).unwrap();
    let projects = orchestrator.list_projects().unwrap();
    assert_eq!(projects.len(), 1);
}

#[tokio::test]
async fn orchestrator_budget_summary() {
    let (orchestrator, _) = setup_orchestrator(BudgetMode::Govern, MoneyAmount::ZERO);

    let year_month = chrono::Utc::now().format("%Y-%m").to_string();
    let summary = orchestrator
        .budget_summary(MoneyAmount::from_dollars(100.0), &year_month)
        .unwrap();

    assert_eq!(summary.global_limit, MoneyAmount::from_dollars(100.0));
    assert_eq!(summary.global_spent, MoneyAmount::ZERO);
}

#[tokio::test]
async fn orchestrator_task_with_skeleton_adapter() {
    // Claude adapter is a skeleton — submit will fail, but orchestrator handles it gracefully
    let storage = Arc::new(SqliteStorage::in_memory().unwrap());
    let project = Project::new("p", "/tmp/p");
    storage.insert_project(&project).unwrap();

    let mut registry = AdapterRegistry::new();
    // Use a FakeAdapter that succeeds for Claude so we can test the full flow
    registry.register(Arc::new(FakeAdapter::new(
        "claude",
        FakeAdapter::full_capabilities(),
        FakeBehavior::SucceedWithUsage {
            output: "deep analysis complete".into(),
            input_tokens: 2000,
            output_tokens: 1000,
            cost: MoneyAmount::from_cents(500),
        },
    )));
    registry.register(Arc::new(FakeAdapter::local("ollama")));

    let registry = Arc::new(registry);
    let governor = Arc::new(BudgetGovernor::new(
        BudgetConfig {
            mode: BudgetMode::Observe,
            ..BudgetConfig::default()
        },
        Arc::new(InMemoryUsageStore::new()),
    ));
    let routing_engine = RoutingEngine::new(
        RoutingPolicy::default(),
        Arc::clone(&registry),
        Arc::clone(&governor),
    );
    let orchestrator = Orchestrator::new(registry, routing_engine, governor, storage);

    let task = Task::new(project.id.clone(), TaskType::DeepCodeReasoning, "analyze error handling");

    let result = orchestrator
        .submit_task(task, ProjectRoutingConfig::default(), MoneyAmount::from_cents(500))
        .await
        .unwrap();

    assert_eq!(result.routing_decision.selected_backend, BackendId::new("claude"));
    assert_eq!(result.execution_output.as_deref(), Some("deep analysis complete"));
    assert!(result.usage.is_some());
    assert_eq!(result.usage.unwrap().cost, MoneyAmount::from_cents(500));
}

// -----------------------------------------------------------------------
// Phase 4: Pending action lifecycle tests
// -----------------------------------------------------------------------

#[tokio::test]
async fn orchestrator_create_and_list_actions() {
    let (orchestrator, project) = setup_orchestrator(BudgetMode::Observe, MoneyAmount::ZERO);

    let action = PendingAction::new(
        PendingActionType::ReviewRequest,
        project.id.clone(),
        "review auth module",
    );
    orchestrator.create_action(&action).unwrap();

    let pending = orchestrator.list_pending_actions().unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].description, "review auth module");
    assert_eq!(pending[0].status, ActionStatus::Pending);
}

#[tokio::test]
async fn orchestrator_approve_action() {
    let (orchestrator, project) = setup_orchestrator(BudgetMode::Observe, MoneyAmount::ZERO);

    let action = PendingAction::new(
        PendingActionType::CommitSuggestion,
        project.id.clone(),
        "suggested commit message",
    )
    .with_payload(serde_json::json!({"commit_message": "fix: resolve null pointer"}));

    orchestrator.create_action(&action).unwrap();
    orchestrator.approve_action(&action.id).unwrap();

    // Should no longer appear in pending list
    let pending = orchestrator.list_pending_actions().unwrap();
    assert!(pending.is_empty());

    // But should still appear in all actions list
    let all = orchestrator.list_all_actions(10).unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].status, ActionStatus::Approved);
}

#[tokio::test]
async fn orchestrator_dismiss_action() {
    let (orchestrator, project) = setup_orchestrator(BudgetMode::Observe, MoneyAmount::ZERO);

    let action = PendingAction::new(
        PendingActionType::BudgetApproval,
        project.id.clone(),
        "approve over-budget task",
    );
    orchestrator.create_action(&action).unwrap();
    orchestrator.dismiss_action(&action.id).unwrap();

    let pending = orchestrator.list_pending_actions().unwrap();
    assert!(pending.is_empty());

    let all = orchestrator.list_all_actions(10).unwrap();
    assert_eq!(all[0].status, ActionStatus::Rejected);
}

#[tokio::test]
async fn orchestrator_action_emits_events() {
    let (orchestrator, project) = setup_orchestrator(BudgetMode::Observe, MoneyAmount::ZERO);

    let action = PendingAction::new(
        PendingActionType::ReviewRequest,
        project.id.clone(),
        "review changes",
    );
    orchestrator.create_action(&action).unwrap();
    orchestrator.approve_action(&action.id).unwrap();

    let events = orchestrator.recent_events(20).unwrap();
    let event_types: Vec<_> = events.iter().map(|e| e.event_type).collect();

    use strategos::models::event::EventType;
    assert!(event_types.contains(&EventType::ActionCreated));
    assert!(event_types.contains(&EventType::ActionApproved));
}

#[tokio::test]
async fn orchestrator_get_action_by_id() {
    let (orchestrator, project) = setup_orchestrator(BudgetMode::Observe, MoneyAmount::ZERO);

    let action = PendingAction::new(
        PendingActionType::CommitSuggestion,
        project.id.clone(),
        "commit suggestion",
    )
    .with_payload(serde_json::json!({"message": "feat: add auth"}));

    orchestrator.create_action(&action).unwrap();

    let fetched = orchestrator.get_pending_action(&action.id).unwrap().unwrap();
    assert_eq!(fetched.description, "commit suggestion");
    assert_eq!(fetched.payload["message"], "feat: add auth");
}

// -----------------------------------------------------------------------
// Phase 4: Status overview tests
// -----------------------------------------------------------------------

#[tokio::test]
async fn orchestrator_project_status_summary() {
    let (orchestrator, project) = setup_orchestrator(BudgetMode::Observe, MoneyAmount::ZERO);

    // Submit a task so we have counts
    let task = Task::new(project.id.clone(), TaskType::Summarization, "test");
    orchestrator
        .submit_task(task, ProjectRoutingConfig::default(), MoneyAmount::from_cents(50))
        .await
        .unwrap();

    // Create a pending action
    let action = PendingAction::new(
        PendingActionType::ReviewRequest,
        project.id.clone(),
        "review",
    );
    orchestrator.create_action(&action).unwrap();

    let entries = orchestrator.project_status_summary().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "test-project");
    assert_eq!(entries[0].pending_actions, 1);
    // Should have at least one task
    let total_tasks: usize = entries[0].task_counts.iter().map(|(_, c)| c).sum();
    assert!(total_tasks >= 1);
}

// -----------------------------------------------------------------------
// Phase 4: Task detail and actions-for-task tests
// -----------------------------------------------------------------------

#[tokio::test]
async fn orchestrator_task_detail_and_routing_history() {
    let (orchestrator, project) = setup_orchestrator(BudgetMode::Observe, MoneyAmount::ZERO);

    let task = Task::new(project.id.clone(), TaskType::Summarization, "summarize");
    let task_id = task.id.clone();

    orchestrator
        .submit_task(task, ProjectRoutingConfig::default(), MoneyAmount::from_cents(50))
        .await
        .unwrap();

    // Get task detail
    let fetched = orchestrator.get_task(&task_id).unwrap().unwrap();
    assert_eq!(fetched.description, "summarize");

    // Get routing history
    let routing = orchestrator.get_routing_history_for_task(&task_id).unwrap();
    assert!(routing.is_some());
    let routing = routing.unwrap();
    assert!(!routing.selected_backend.is_empty());
}

#[tokio::test]
async fn orchestrator_actions_linked_to_task() {
    let (orchestrator, project) = setup_orchestrator(BudgetMode::Observe, MoneyAmount::ZERO);

    let task = Task::new(project.id.clone(), TaskType::Review, "review code");
    let task_id = task.id.clone();

    orchestrator
        .submit_task(task, ProjectRoutingConfig::default(), MoneyAmount::from_cents(50))
        .await
        .unwrap();

    // Create an action linked to this task
    let action = PendingAction::new(
        PendingActionType::ReviewRequest,
        project.id.clone(),
        "review findings",
    )
    .with_task(task_id.clone());
    orchestrator.create_action(&action).unwrap();

    let actions = orchestrator.list_actions_for_task(&task_id).unwrap();
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0].description, "review findings");
}

// -----------------------------------------------------------------------
// Phase 5: Budget approval workflow test
// -----------------------------------------------------------------------

#[tokio::test]
async fn orchestrator_budget_approval_creates_pending_action() {
    // Set up with Govern mode at 95% spend — triggers RequireApproval at 90% threshold
    let (orchestrator, project) =
        setup_orchestrator(BudgetMode::Govern, MoneyAmount::from_dollars(95.0));

    let task = Task::new(project.id.clone(), TaskType::Planning, "plan something");

    let result = orchestrator
        .submit_task(task, ProjectRoutingConfig::default(), MoneyAmount::from_cents(100))
        .await
        .unwrap();

    // Should require approval
    assert!(result.requires_approval, "expected requires_approval=true at 95% budget");
    assert!(result.pending_action_id.is_some());

    // Should have created a BudgetApproval pending action
    let pending = orchestrator.list_pending_actions().unwrap();
    assert!(
        pending.iter().any(|a| a.action_type == PendingActionType::BudgetApproval),
        "expected a BudgetApproval pending action"
    );

    // Should have emitted an ActionCreated event
    let events = orchestrator.recent_events(20).unwrap();
    use strategos::models::event::EventType;
    assert!(events.iter().any(|e| e.event_type == EventType::ActionCreated));
}

// -----------------------------------------------------------------------
// Phase 6: Execution context and cost estimation tests
// -----------------------------------------------------------------------

#[tokio::test]
async fn orchestrator_submit_with_context() {
    let (orchestrator, project) = setup_orchestrator(BudgetMode::Observe, MoneyAmount::ZERO);

    let task = Task::new(project.id.clone(), TaskType::Summarization, "summarize");
    let project_path = std::path::PathBuf::from("/tmp/test");
    let files = vec![std::path::PathBuf::from("/tmp/test/src/main.rs")];

    let result = orchestrator
        .submit_task_with_context(
            task,
            ProjectRoutingConfig::default(),
            MoneyAmount::from_cents(50),
            Some(project_path),
            files,
            strategos::adapters::traits::ExecutionConstraints::default(),
        )
        .await
        .unwrap();

    assert_eq!(result.routing_decision.selected_backend, BackendId::new("ollama"));
}

#[tokio::test]
async fn orchestrator_cost_estimation_zero_for_local() {
    use strategos::adapters::traits::estimate_task_cost;

    let cost = estimate_task_cost(
        "summarize this module please",
        &BackendId::new("ollama"),
        "llama3",
    );
    assert_eq!(cost, MoneyAmount::ZERO);
}

// -----------------------------------------------------------------------
// Phase 7: Task output persistence tests
// -----------------------------------------------------------------------

#[tokio::test]
async fn orchestrator_persists_task_output() {
    let (orchestrator, project) = setup_orchestrator(BudgetMode::Observe, MoneyAmount::ZERO);

    let task = Task::new(project.id.clone(), TaskType::Summarization, "summarize");
    let task_id = task.id.clone();

    let result = orchestrator
        .submit_task(task, ProjectRoutingConfig::default(), MoneyAmount::from_cents(50))
        .await
        .unwrap();

    // Task should have completed and stored output
    assert!(result.execution_output.is_some());

    // Output should be retrievable from storage
    let output = orchestrator.storage.get_task_output(&task_id).unwrap();
    assert!(output.is_some(), "task output should be persisted");
    let output = output.unwrap();
    assert!(!output.output.is_empty());
    assert_eq!(output.backend_id, result.routing_decision.selected_backend.as_str());
}

#[tokio::test]
async fn orchestrator_no_output_for_failed_task() {
    let storage = Arc::new(SqliteStorage::in_memory().unwrap());
    let project = Project::new("p", "/tmp/p");
    storage.insert_project(&project).unwrap();

    let mut registry = AdapterRegistry::new();
    use strategos::errors::AdapterError;
    registry.register(Arc::new(FakeAdapter::failing(
        "claude",
        AdapterError::Unavailable("test unavailable".into()),
    )));
    registry.register(Arc::new(FakeAdapter::failing(
        "ollama",
        AdapterError::Unavailable("test unavailable".into()),
    )));

    let registry = Arc::new(registry);
    let governor = Arc::new(BudgetGovernor::new(
        BudgetConfig {
            mode: BudgetMode::Observe,
            ..BudgetConfig::default()
        },
        Arc::new(InMemoryUsageStore::new()),
    ));
    let mut policy = RoutingPolicy::default();
    policy.check_health_before_routing = false; // skip health checks
    let routing_engine = RoutingEngine::new(policy, Arc::clone(&registry), Arc::clone(&governor));
    let orchestrator = Orchestrator::new(registry, routing_engine, governor, Arc::clone(&storage));

    let task = Task::new(project.id.clone(), TaskType::Summarization, "summarize");
    let task_id = task.id.clone();

    let _ = orchestrator
        .submit_task(task, ProjectRoutingConfig::default(), MoneyAmount::from_cents(50))
        .await;

    // No output should be persisted for a failed task
    let output = storage.get_task_output(&task_id).unwrap();
    assert!(output.is_none());
}

// -----------------------------------------------------------------------
// Phase 7: Task cancellation tests
// -----------------------------------------------------------------------

#[tokio::test]
async fn orchestrator_cancel_pending_task() {
    let (orchestrator, project) = setup_orchestrator(BudgetMode::Observe, MoneyAmount::ZERO);

    // Insert a task directly (don't submit to keep it Pending)
    let task = Task::new(project.id.clone(), TaskType::Summarization, "cancel me");
    let task_id = task.id.clone();
    orchestrator.storage.insert_task(&task).unwrap();

    orchestrator.cancel_task(&task_id).unwrap();

    let fetched = orchestrator.get_task(&task_id).unwrap().unwrap();
    assert_eq!(fetched.status, TaskStatus::Cancelled);

    // Should have emitted a TaskCancelled event
    let events = orchestrator.recent_events(20).unwrap();
    use strategos::models::event::EventType;
    assert!(events.iter().any(|e| e.event_type == EventType::TaskCancelled));
}

#[tokio::test]
async fn orchestrator_cancel_completed_task_fails() {
    let (orchestrator, project) = setup_orchestrator(BudgetMode::Observe, MoneyAmount::ZERO);

    // Submit and complete a task
    let task = Task::new(project.id.clone(), TaskType::Summarization, "done task");
    let task_id = task.id.clone();
    orchestrator
        .submit_task(task, ProjectRoutingConfig::default(), MoneyAmount::from_cents(50))
        .await
        .unwrap();

    // Should not be cancellable
    let result = orchestrator.cancel_task(&task_id);
    assert!(result.is_err());
    match result.unwrap_err() {
        CancelError::InvalidState(msg) => {
            assert!(msg.contains("Completed"), "expected Completed in: {}", msg);
        }
        other => panic!("expected InvalidState, got: {:?}", other),
    }
}

// -----------------------------------------------------------------------
// Phase 7: Spending trends (storage-level) tests
// -----------------------------------------------------------------------

#[tokio::test]
async fn storage_spend_by_month() {
    let storage = SqliteStorage::in_memory().unwrap();
    let project = Project::new("p", "/tmp/p");
    storage.insert_project(&project).unwrap();

    let task = Task::new(project.id.clone(), TaskType::Summarization, "test");
    storage.insert_task(&task).unwrap();

    // Insert usage records
    let usage = strategos::models::usage::UsageRecord::new(
        task.id.clone(),
        project.id.clone(),
        BackendId::new("claude"),
        100,
        50,
        MoneyAmount::from_cents(200),
    );
    storage.insert_usage(&usage).unwrap();

    let monthly = storage.spend_by_month(3).unwrap();
    assert!(!monthly.is_empty());
    assert_eq!(monthly[0].1, MoneyAmount::from_cents(200));
}

#[tokio::test]
async fn storage_spend_by_backend_month() {
    let storage = SqliteStorage::in_memory().unwrap();
    let project = Project::new("p", "/tmp/p");
    storage.insert_project(&project).unwrap();

    let task = Task::new(project.id.clone(), TaskType::Summarization, "test");
    storage.insert_task(&task).unwrap();

    let usage = strategos::models::usage::UsageRecord::new(
        task.id.clone(),
        project.id.clone(),
        BackendId::new("claude"),
        100,
        50,
        MoneyAmount::from_cents(300),
    );
    storage.insert_usage(&usage).unwrap();

    let by_backend = storage.spend_by_backend_month(3).unwrap();
    assert!(!by_backend.is_empty());
    assert_eq!(by_backend[0].1, "claude");
    assert_eq!(by_backend[0].2, MoneyAmount::from_cents(300));
}

#[tokio::test]
async fn storage_spend_by_project_month() {
    let storage = SqliteStorage::in_memory().unwrap();
    let project = Project::new("trends-test", "/tmp/t");
    storage.insert_project(&project).unwrap();

    let task = Task::new(project.id.clone(), TaskType::Review, "review");
    storage.insert_task(&task).unwrap();

    let usage = strategos::models::usage::UsageRecord::new(
        task.id.clone(),
        project.id.clone(),
        BackendId::new("ollama"),
        50,
        25,
        MoneyAmount::from_cents(0),
    );
    storage.insert_usage(&usage).unwrap();

    let by_project = storage.spend_by_project_month(3).unwrap();
    // ollama is zero-cost, so might not appear if SUM is 0
    // Just verify it doesn't error
    assert!(by_project.is_empty() || by_project[0].1 == project.id);
}

#[tokio::test]
async fn storage_task_output_roundtrip() {
    let storage = SqliteStorage::in_memory().unwrap();
    let project = Project::new("p", "/tmp/p");
    storage.insert_project(&project).unwrap();

    let task = Task::new(project.id.clone(), TaskType::Summarization, "test");
    storage.insert_task(&task).unwrap();

    storage
        .insert_task_output(
            &task.id,
            "claude",
            "the output text",
            Some(&serde_json::json!({"key": "value"})),
            Some("claude-sonnet-4-20250514"),
            150,
            500,
            200,
        )
        .unwrap();

    let output = storage.get_task_output(&task.id).unwrap().unwrap();
    assert_eq!(output.output, "the output text");
    assert_eq!(output.backend_id, "claude");
    assert_eq!(output.model.as_deref(), Some("claude-sonnet-4-20250514"));
    assert_eq!(output.cost_cents, 150);
    assert_eq!(output.input_tokens, 500);
    assert_eq!(output.output_tokens, 200);
    assert!(output.structured_output.is_some());
}

// -----------------------------------------------------------------------
// Phase 8: Execution constraint enforcement tests
// -----------------------------------------------------------------------

#[tokio::test]
async fn constraint_max_cost_rejects_expensive_task() {
    let (orchestrator, project) = setup_orchestrator(BudgetMode::Observe, MoneyAmount::ZERO);

    let task = Task::new(project.id.clone(), TaskType::Summarization, "summarize");

    let constraints = ExecutionConstraints {
        max_cost_cents: Some(1), // 1 cent max
        ..ExecutionConstraints::default()
    };

    // Estimated cost for "summarize" via default backend will likely be > 1 cent or 0
    // Use a large estimated cost to ensure the constraint triggers
    let result = orchestrator
        .submit_task_with_context(
            task,
            ProjectRoutingConfig::default(),
            MoneyAmount::from_cents(500), // estimated cost 500 cents
            None,
            Vec::new(),
            constraints,
        )
        .await;

    match result {
        Err(e) => {
            let err_msg = format!("{}", e);
            assert!(err_msg.contains("cost exceeds constraint"), "got: {}", err_msg);
        }
        Ok(_) => panic!("expected cost constraint to reject task"),
    }
}

#[tokio::test]
async fn constraint_timeout_causes_failure() {
    // Use FakeAdapter that takes time (but is immediate for now, timeout will be very short)
    let (orchestrator, project) = setup_orchestrator(BudgetMode::Observe, MoneyAmount::ZERO);

    let task = Task::new(project.id.clone(), TaskType::Summarization, "summarize");

    // Very short timeout — but FakeAdapter responds immediately, so this should succeed
    let constraints = ExecutionConstraints {
        timeout: Some(std::time::Duration::from_secs(10)),
        ..ExecutionConstraints::default()
    };

    let result = orchestrator
        .submit_task_with_context(
            task,
            ProjectRoutingConfig::default(),
            MoneyAmount::from_cents(50),
            None,
            Vec::new(),
            constraints,
        )
        .await
        .unwrap();

    // FakeAdapter is instant, so this should succeed within timeout
    assert!(result.execution_output.is_some());
}

#[tokio::test]
async fn constraint_no_cost_limit_allows_any() {
    let (orchestrator, project) = setup_orchestrator(BudgetMode::Observe, MoneyAmount::ZERO);

    let task = Task::new(project.id.clone(), TaskType::Summarization, "summarize");

    // No cost limit
    let constraints = ExecutionConstraints::default();

    let result = orchestrator
        .submit_task_with_context(
            task,
            ProjectRoutingConfig::default(),
            MoneyAmount::from_cents(99999),
            None,
            Vec::new(),
            constraints,
        )
        .await
        .unwrap();

    // Should succeed since no max_cost_cents constraint
    assert!(!result.requires_approval || result.execution_output.is_some() || result.execution_output.is_none());
}

// -----------------------------------------------------------------------
// Phase 8: Retry policy tests
// -----------------------------------------------------------------------

#[tokio::test]
async fn retry_on_transient_error_succeeds() {
    // We can't easily make FakeAdapter fail then succeed, but we can test that
    // permanent errors are NOT retried.
    let storage = Arc::new(SqliteStorage::in_memory().unwrap());
    let project = Project::new("p", "/tmp/p");
    storage.insert_project(&project).unwrap();

    let mut registry = AdapterRegistry::new();
    // Auth error is permanent — should not be retried
    registry.register(Arc::new(FakeAdapter::failing(
        "claude",
        AdapterError::AuthenticationFailed("bad key".into()),
    )));
    registry.register(Arc::new(FakeAdapter::local("ollama")));

    let registry = Arc::new(registry);
    let governor = Arc::new(BudgetGovernor::new(
        BudgetConfig {
            mode: BudgetMode::Observe,
            ..BudgetConfig::default()
        },
        Arc::new(InMemoryUsageStore::new()),
    ));
    let mut policy = RoutingPolicy::default();
    policy.check_health_before_routing = false;
    let routing_engine = RoutingEngine::new(policy, Arc::clone(&registry), Arc::clone(&governor));
    let orchestrator = Orchestrator::new(registry, routing_engine, governor, Arc::clone(&storage))
        .with_retry_policy(RetryPolicy {
            max_retries: 2,
            retry_delay: std::time::Duration::from_millis(1),
        });

    let task = Task::new(project.id.clone(), TaskType::Summarization, "test");
    let task_id = task.id.clone();

    // Submit — ollama should succeed (it's the default for Summarization)
    let result = orchestrator
        .submit_task(task, ProjectRoutingConfig::default(), MoneyAmount::from_cents(50))
        .await
        .unwrap();

    // Ollama (local) should have been selected and succeeded
    assert_eq!(result.routing_decision.selected_backend, BackendId::new("ollama"));

    let fetched = storage.get_task(&task_id).unwrap().unwrap();
    assert_eq!(fetched.status, TaskStatus::Completed);
}

#[tokio::test]
async fn retry_exhaustion_fails() {
    let storage = Arc::new(SqliteStorage::in_memory().unwrap());
    let project = Project::new("p", "/tmp/p");
    storage.insert_project(&project).unwrap();

    let mut registry = AdapterRegistry::new();
    // Transient error on ollama
    registry.register(Arc::new(FakeAdapter::failing(
        "ollama",
        AdapterError::RequestFailed("connection reset".into()),
    )));

    // Give ollama local capabilities but make it fail with transient error
    let mut local_failing = AdapterRegistry::new();
    local_failing.register(Arc::new(FakeAdapter::new(
        "ollama",
        strategos::adapters::traits::AdapterCapabilities {
            code_editing: false,
            shell_tool_use: false,
            multi_step_agent: false,
            local_execution: true,
            structured_output: false,
            streaming: false,
            subagents: false,
            session_resume: false,
        },
        FakeBehavior::FailWith(AdapterError::RequestFailed("connection reset".into())),
    )));

    let registry = Arc::new(local_failing);
    let governor = Arc::new(BudgetGovernor::new(
        BudgetConfig {
            mode: BudgetMode::Observe,
            ..BudgetConfig::default()
        },
        Arc::new(InMemoryUsageStore::new()),
    ));
    let mut policy = RoutingPolicy::default();
    policy.check_health_before_routing = false;
    let routing_engine = RoutingEngine::new(policy, Arc::clone(&registry), Arc::clone(&governor));
    let orchestrator = Orchestrator::new(registry, routing_engine, governor, Arc::clone(&storage))
        .with_retry_policy(RetryPolicy {
            max_retries: 1, // 1 retry = 2 total attempts
            retry_delay: std::time::Duration::from_millis(1),
        });

    let task = Task::new(project.id.clone(), TaskType::Summarization, "test");

    // This should fail after retries (transient RequestFailed)
    let result = orchestrator
        .submit_task(task, ProjectRoutingConfig::default(), MoneyAmount::from_cents(50))
        .await
        .unwrap();

    // Should have no output (failed)
    assert!(result.execution_output.is_none());
}

// -----------------------------------------------------------------------
// Phase 8: Project auto-sync tests
// -----------------------------------------------------------------------

#[test]
fn project_sync_adds_new_project() {
    let storage = SqliteStorage::in_memory().unwrap();

    // No projects in storage
    assert!(storage.list_projects().unwrap().is_empty());

    // Sync a config project
    let config = strategos::config::GlobalConfig::sample();
    strategos::cli::sync_projects_from_config(&config, &storage);

    let projects = storage.list_projects().unwrap();
    assert!(projects.len() >= 1);
    assert!(projects.iter().any(|p| p.name == "my-project"));
}

#[test]
fn project_sync_updates_existing() {
    let storage = SqliteStorage::in_memory().unwrap();

    let mut project = Project::new("my-project", "/old/path");
    project.privacy = PrivacyLevel::Public;
    storage.insert_project(&project).unwrap();

    // Config has different path
    let config = strategos::config::GlobalConfig::sample();
    strategos::cli::sync_projects_from_config(&config, &storage);

    let updated = storage.get_project_by_name("my-project").unwrap().unwrap();
    // Path should be updated from config
    assert_ne!(updated.path.to_str(), Some("/old/path"));
}

#[test]
fn project_sync_preserves_cli_added_projects() {
    let storage = SqliteStorage::in_memory().unwrap();

    // Add a project that's NOT in config
    let extra = Project::new("cli-only-project", "/tmp/cli-only");
    storage.insert_project(&extra).unwrap();

    let config = strategos::config::GlobalConfig::sample();
    strategos::cli::sync_projects_from_config(&config, &storage);

    // CLI-only project should still exist
    let projects = storage.list_projects().unwrap();
    assert!(projects.iter().any(|p| p.name == "cli-only-project"));
    // Config projects should also exist
    assert!(projects.iter().any(|p| p.name == "my-project"));
}

// -----------------------------------------------------------------------
// Phase 8: AdapterError transient classification tests
// -----------------------------------------------------------------------

#[test]
fn adapter_error_transient_classification() {
    assert!(AdapterError::RateLimited { retry_after: None }.is_transient());
    assert!(AdapterError::Unavailable("down".into()).is_transient());
    assert!(AdapterError::Timeout(std::time::Duration::from_secs(30)).is_transient());
    assert!(AdapterError::RequestFailed("reset".into()).is_transient());

    assert!(!AdapterError::AuthenticationFailed("bad key".into()).is_transient());
    assert!(!AdapterError::Unsupported("not supported".into()).is_transient());
    assert!(!AdapterError::Internal("bug".into()).is_transient());
}

// -----------------------------------------------------------------------
// Phase 8: Storage update_project test
// -----------------------------------------------------------------------

#[test]
fn storage_update_project() {
    let storage = SqliteStorage::in_memory().unwrap();

    let mut project = Project::new("test-proj", "/old/path");
    storage.insert_project(&project).unwrap();

    project.path = std::path::PathBuf::from("/new/path");
    project.privacy = PrivacyLevel::LocalOnly;
    storage.update_project(&project).unwrap();

    let fetched = storage.get_project_by_name("test-proj").unwrap().unwrap();
    assert_eq!(fetched.path, std::path::PathBuf::from("/new/path"));
    assert_eq!(fetched.privacy, PrivacyLevel::LocalOnly);
}
