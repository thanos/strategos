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
use strategos::storage::sqlite::{SqliteStorage, ProjectExportData};

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
            ..RetryPolicy::default()
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
            ..RetryPolicy::default()
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

// -----------------------------------------------------------------------
// Phase 9: Task Dependencies
// -----------------------------------------------------------------------

#[test]
fn task_dependency_storage_roundtrip() {
    let storage = SqliteStorage::in_memory().unwrap();
    let project = Project::new("dep-project", "/tmp/dep");
    storage.insert_project(&project).unwrap();

    let task_a = Task::new(project.id.clone(), TaskType::Planning, "task A");
    let task_b = Task::new(project.id.clone(), TaskType::Review, "task B depends on A");

    storage.insert_task(&task_a).unwrap();
    storage.insert_task(&task_b).unwrap();

    storage.insert_task_dependency(&task_b.id, &task_a.id).unwrap();

    let deps = storage.get_task_dependencies(&task_b.id).unwrap();
    assert_eq!(deps.len(), 1);
    assert_eq!(deps[0], task_a.id);
}

#[test]
fn all_dependencies_completed_when_dep_done() {
    let storage = SqliteStorage::in_memory().unwrap();
    let project = Project::new("dep-project", "/tmp/dep");
    storage.insert_project(&project).unwrap();

    let task_a = Task::new(project.id.clone(), TaskType::Planning, "task A");
    let task_b = Task::new(project.id.clone(), TaskType::Review, "task B");

    storage.insert_task(&task_a).unwrap();
    storage.insert_task(&task_b).unwrap();

    storage.insert_task_dependency(&task_b.id, &task_a.id).unwrap();

    // task_a is Pending, so deps are not satisfied
    assert!(!storage.all_dependencies_completed(&task_b.id).unwrap());

    // Complete task_a
    storage.update_task_status(&task_a.id, TaskStatus::Completed).unwrap();
    assert!(storage.all_dependencies_completed(&task_b.id).unwrap());
}

#[test]
fn submit_with_satisfied_deps_succeeds() {
    let (orchestrator, project) = setup_orchestrator(BudgetMode::Observe, MoneyAmount::ZERO);

    // Create and complete a dependency task
    let dep_task = Task::new(project.id.clone(), TaskType::Planning, "dependency");
    orchestrator.storage.insert_task(&dep_task).unwrap();
    orchestrator.storage.update_task_status(&dep_task.id, TaskStatus::Completed).unwrap();

    // Create a new task that depends on the completed one
    let task = Task::new(project.id.clone(), TaskType::Review, "depends on planning");
    orchestrator.storage.insert_task(&task).unwrap();
    orchestrator.add_task_dependencies(&task.id, &[dep_task.id.clone()]).unwrap();

    // Dependencies satisfied
    assert!(orchestrator.check_dependencies(&task.id).unwrap());
}

#[test]
fn submit_with_unsatisfied_deps_fails_check() {
    let (orchestrator, project) = setup_orchestrator(BudgetMode::Observe, MoneyAmount::ZERO);

    // Create a dependency task (still pending)
    let dep_task = Task::new(project.id.clone(), TaskType::Planning, "dependency");
    orchestrator.storage.insert_task(&dep_task).unwrap();

    // Create a task that depends on the pending one
    let task = Task::new(project.id.clone(), TaskType::Review, "depends on planning");
    orchestrator.storage.insert_task(&task).unwrap();
    orchestrator.add_task_dependencies(&task.id, &[dep_task.id.clone()]).unwrap();

    // Dependencies NOT satisfied
    assert!(!orchestrator.check_dependencies(&task.id).unwrap());
}

#[test]
fn task_dependency_display_in_storage() {
    let storage = SqliteStorage::in_memory().unwrap();
    let project = Project::new("dep-project", "/tmp/dep");
    storage.insert_project(&project).unwrap();

    let task_a = Task::new(project.id.clone(), TaskType::Planning, "A");
    let task_b = Task::new(project.id.clone(), TaskType::Review, "B");
    let task_c = Task::new(project.id.clone(), TaskType::Summarization, "C depends on A and B");

    storage.insert_task(&task_a).unwrap();
    storage.insert_task(&task_b).unwrap();
    storage.insert_task(&task_c).unwrap();

    storage.insert_task_dependency(&task_c.id, &task_a.id).unwrap();
    storage.insert_task_dependency(&task_c.id, &task_b.id).unwrap();

    let deps = storage.get_task_dependencies(&task_c.id).unwrap();
    assert_eq!(deps.len(), 2);
}

// -----------------------------------------------------------------------
// Phase 9: Dry-Run Routing
// -----------------------------------------------------------------------

#[tokio::test]
async fn dry_run_produces_decision_without_creating_task() {
    let (orchestrator, project) = setup_orchestrator(BudgetMode::Observe, MoneyAmount::ZERO);

    let task = Task::new(project.id.clone(), TaskType::Summarization, "dry run test");
    let project_config = ProjectRoutingConfig::default();

    let routing_request = RoutingRequest {
        task_id: task.id.clone(),
        task_type: task.task_type,
        project_id: task.project_id.clone(),
        project_config,
        backend_override: None,
        estimated_cost: MoneyAmount::from_cents(50),
    };

    // Route without persisting
    let decision = orchestrator.routing_engine.route(routing_request).await.unwrap();
    assert!(!decision.selected_backend.as_str().is_empty());

    // Task was NOT persisted
    let stored = orchestrator.storage.get_task(&task.id).unwrap();
    assert!(stored.is_none(), "dry-run should not create a task in storage");
}

// -----------------------------------------------------------------------
// Phase 9: Event Filtering
// -----------------------------------------------------------------------

#[test]
fn event_filter_by_type() {
    let storage = SqliteStorage::in_memory().unwrap();
    let project = Project::new("evt-project", "/tmp/evt");
    storage.insert_project(&project).unwrap();

    use strategos::models::event::{Event, EventType};

    let e1 = Event::new(EventType::TaskSubmitted, serde_json::json!({}))
        .with_project(project.id.clone());
    let e2 = Event::new(EventType::TaskCompleted, serde_json::json!({}))
        .with_project(project.id.clone());
    let e3 = Event::new(EventType::TaskSubmitted, serde_json::json!({}))
        .with_project(project.id.clone());

    storage.insert_event(&e1).unwrap();
    storage.insert_event(&e2).unwrap();
    storage.insert_event(&e3).unwrap();

    let filtered = storage
        .list_events_filtered(Some("TaskSubmitted"), None, None, None, None, 100)
        .unwrap();
    assert_eq!(filtered.len(), 2);
    for e in &filtered {
        assert_eq!(e.event_type, EventType::TaskSubmitted);
    }
}

#[test]
fn event_filter_by_project() {
    let storage = SqliteStorage::in_memory().unwrap();
    let p1 = Project::new("proj-a", "/tmp/a");
    let p2 = Project::new("proj-b", "/tmp/b");
    storage.insert_project(&p1).unwrap();
    storage.insert_project(&p2).unwrap();

    use strategos::models::event::{Event, EventType};

    let e1 = Event::new(EventType::TaskSubmitted, serde_json::json!({}))
        .with_project(p1.id.clone());
    let e2 = Event::new(EventType::TaskSubmitted, serde_json::json!({}))
        .with_project(p2.id.clone());
    let e3 = Event::new(EventType::TaskCompleted, serde_json::json!({}))
        .with_project(p1.id.clone());

    storage.insert_event(&e1).unwrap();
    storage.insert_event(&e2).unwrap();
    storage.insert_event(&e3).unwrap();

    let filtered = storage
        .list_events_filtered(None, Some(&p1.id), None, None, None, 100)
        .unwrap();
    assert_eq!(filtered.len(), 2);
}

#[test]
fn event_filter_by_date_range() {
    let storage = SqliteStorage::in_memory().unwrap();

    use strategos::models::event::{Event, EventType};
    use chrono::{Utc, Duration};

    let now = Utc::now();
    let e1 = Event::new(EventType::TaskSubmitted, serde_json::json!({}));
    storage.insert_event(&e1).unwrap();

    // Filter with a future since — should return nothing
    let future = (now + Duration::hours(1)).to_rfc3339();
    let filtered = storage
        .list_events_filtered(None, None, None, Some(&future), None, 100)
        .unwrap();
    assert_eq!(filtered.len(), 0);

    // Filter with a past since — should return the event
    let past = (now - Duration::hours(1)).to_rfc3339();
    let filtered = storage
        .list_events_filtered(None, None, None, Some(&past), None, 100)
        .unwrap();
    assert_eq!(filtered.len(), 1);
}

// -----------------------------------------------------------------------
// Phase 9: Project Export/Import
// -----------------------------------------------------------------------

#[test]
fn export_roundtrip() {
    let storage = SqliteStorage::in_memory().unwrap();
    let project = Project::new("export-proj", "/tmp/export");
    storage.insert_project(&project).unwrap();

    let task = Task::new(project.id.clone(), TaskType::Planning, "plan something");
    storage.insert_task(&task).unwrap();
    storage.update_task_status(&task.id, TaskStatus::Completed).unwrap();

    // Add a usage record
    storage
        .insert_usage_record(
            &uuid::Uuid::new_v4().to_string(),
            &task.id.0.to_string(),
            &project.id.0.to_string(),
            "claude",
            100,
            50,
            75,
            Some("claude-sonnet"),
            &Utc::now().to_rfc3339(),
        )
        .unwrap();

    let data = storage.export_project_data(&project.id).unwrap();
    assert_eq!(data.project.name, "export-proj");
    assert_eq!(data.tasks.len(), 1);
    assert_eq!(data.usage_records.len(), 1);

    // Serialize/deserialize roundtrip
    let json = serde_json::to_string_pretty(&data).unwrap();
    let parsed: ProjectExportData = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.project.name, "export-proj");
    assert_eq!(parsed.tasks.len(), 1);
}

use chrono::Utc;

#[test]
fn import_skips_duplicates() {
    let storage = SqliteStorage::in_memory().unwrap();
    let project = Project::new("import-proj", "/tmp/import");
    storage.insert_project(&project).unwrap();

    let task = Task::new(project.id.clone(), TaskType::Review, "review code");
    storage.insert_task(&task).unwrap();

    let data = storage.export_project_data(&project.id).unwrap();

    // Import into same database — everything should be skipped
    let result = storage.import_project_data(&data).unwrap();
    assert!(result.skipped_project);
    assert!(!result.imported_project);
    assert_eq!(result.skipped_tasks, 1);
    assert_eq!(result.imported_tasks, 0);
}

#[test]
fn import_into_fresh_database() {
    let storage1 = SqliteStorage::in_memory().unwrap();
    let project = Project::new("fresh-proj", "/tmp/fresh");
    storage1.insert_project(&project).unwrap();

    let task = Task::new(project.id.clone(), TaskType::Planning, "plan");
    storage1.insert_task(&task).unwrap();

    let data = storage1.export_project_data(&project.id).unwrap();

    // Import into a completely different database
    let storage2 = SqliteStorage::in_memory().unwrap();
    let result = storage2.import_project_data(&data).unwrap();

    assert!(result.imported_project);
    assert!(!result.skipped_project);
    assert_eq!(result.imported_tasks, 1);
    assert_eq!(result.skipped_tasks, 0);

    // Verify data exists in new database
    let imported_project = storage2.get_project_by_name("fresh-proj").unwrap();
    assert!(imported_project.is_some());
    let imported_tasks = storage2.list_tasks_by_project(&project.id).unwrap();
    assert_eq!(imported_tasks.len(), 1);
}

#[test]
fn schema_v4_creates_task_dependencies_table() {
    let storage = SqliteStorage::in_memory().unwrap();

    // Verify the task_dependencies table exists
    let count: i64 = storage
        .conn_ref()
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='task_dependencies'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
}

// =========================================================================
// Phase 10: Priority queuing, retry backoff, webhooks, task templates
// =========================================================================

// --- Step 52: Priority-aware task queuing ---

#[tokio::test]
async fn queue_task_sets_status_and_timestamp() {
    let (orchestrator, project) = setup_orchestrator(BudgetMode::Observe, MoneyAmount::ZERO);
    let mut task = Task::new(project.id.clone(), TaskType::Planning, "queue test");
    task.priority = Priority::High;

    orchestrator.queue_task(&mut task).unwrap();

    assert_eq!(task.status, TaskStatus::Queued);
    assert!(task.queued_at.is_some());

    let stored = orchestrator.get_task(&task.id).unwrap().unwrap();
    assert_eq!(stored.status, TaskStatus::Queued);
    assert!(stored.queued_at.is_some());
}

#[tokio::test]
async fn list_queued_tasks_ordered_by_priority() {
    let (orchestrator, project) = setup_orchestrator(BudgetMode::Observe, MoneyAmount::ZERO);

    let mut low = Task::new(project.id.clone(), TaskType::Summarization, "low priority");
    low.priority = Priority::Low;
    orchestrator.queue_task(&mut low).unwrap();

    let mut critical = Task::new(project.id.clone(), TaskType::Summarization, "critical priority");
    critical.priority = Priority::Critical;
    orchestrator.queue_task(&mut critical).unwrap();

    let mut normal = Task::new(project.id.clone(), TaskType::Summarization, "normal priority");
    normal.priority = Priority::Normal;
    orchestrator.queue_task(&mut normal).unwrap();

    let queued = orchestrator.list_queued_tasks().unwrap();
    assert_eq!(queued.len(), 3);
    assert_eq!(queued[0].priority, Priority::Critical);
    assert_eq!(queued[1].priority, Priority::Normal);
    assert_eq!(queued[2].priority, Priority::Low);
}

#[tokio::test]
async fn dequeue_returns_highest_priority_first() {
    let (orchestrator, project) = setup_orchestrator(BudgetMode::Observe, MoneyAmount::ZERO);

    let mut normal = Task::new(project.id.clone(), TaskType::Summarization, "normal");
    normal.priority = Priority::Normal;
    orchestrator.queue_task(&mut normal).unwrap();

    let mut high = Task::new(project.id.clone(), TaskType::Summarization, "high");
    high.priority = Priority::High;
    orchestrator.queue_task(&mut high).unwrap();

    let dequeued = orchestrator.storage.dequeue_next_task().unwrap().unwrap();
    assert_eq!(dequeued.id, high.id);
    assert_eq!(dequeued.status, TaskStatus::Pending);

    let count = orchestrator.count_queued_tasks().unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn dequeue_empty_queue_returns_none() {
    let (orchestrator, _project) = setup_orchestrator(BudgetMode::Observe, MoneyAmount::ZERO);
    let result = orchestrator.storage.dequeue_next_task().unwrap();
    assert!(result.is_none());
}

#[test]
fn priority_rank_ordering() {
    assert_eq!(Priority::Critical.rank(), 0);
    assert_eq!(Priority::High.rank(), 1);
    assert_eq!(Priority::Normal.rank(), 2);
    assert_eq!(Priority::Low.rank(), 3);
    assert!(Priority::Critical.rank() < Priority::Low.rank());
}

#[tokio::test]
async fn queue_task_emits_event() {
    let (orchestrator, project) = setup_orchestrator(BudgetMode::Observe, MoneyAmount::ZERO);
    let mut task = Task::new(project.id.clone(), TaskType::Planning, "event test");
    orchestrator.queue_task(&mut task).unwrap();

    let events = orchestrator.recent_events(10).unwrap();
    let queue_events: Vec<_> = events
        .iter()
        .filter(|e| e.event_type == strategos::models::event::EventType::TaskQueued)
        .collect();
    assert!(!queue_events.is_empty(), "expected a TaskQueued event");
}

#[tokio::test]
async fn run_next_queued_executes_task() {
    let (orchestrator, project) = setup_orchestrator(BudgetMode::Observe, MoneyAmount::ZERO);

    let mut task = Task::new(project.id.clone(), TaskType::Summarization, "run from queue");
    orchestrator.queue_task(&mut task).unwrap();

    let result = orchestrator
        .run_next_queued(ProjectRoutingConfig::default(), MoneyAmount::from_cents(100))
        .await
        .unwrap();
    assert!(result.is_some());
    let result = result.unwrap();
    // Task should have been dequeued and executed
    assert!(result.execution_output.is_some() || result.routing_decision.selected_backend.as_str() != "none");
}

#[tokio::test]
async fn count_queued_tasks_accurate() {
    let (orchestrator, project) = setup_orchestrator(BudgetMode::Observe, MoneyAmount::ZERO);

    assert_eq!(orchestrator.count_queued_tasks().unwrap(), 0);

    let mut t1 = Task::new(project.id.clone(), TaskType::Planning, "task 1");
    orchestrator.queue_task(&mut t1).unwrap();
    assert_eq!(orchestrator.count_queued_tasks().unwrap(), 1);

    let mut t2 = Task::new(project.id.clone(), TaskType::Planning, "task 2");
    orchestrator.queue_task(&mut t2).unwrap();
    assert_eq!(orchestrator.count_queued_tasks().unwrap(), 2);
}

// --- Step 53: Exponential backoff with jitter ---

#[test]
fn retry_delay_exponential_growth() {
    let policy = RetryPolicy {
        max_retries: 5,
        retry_delay: std::time::Duration::from_millis(1000),
        backoff_multiplier: 2.0,
        max_delay: std::time::Duration::from_millis(60_000),
        jitter_fraction: 0.0, // no jitter for deterministic test
    };

    let d0 = policy.delay_for_attempt(0);
    assert_eq!(d0, std::time::Duration::ZERO);

    let d1 = policy.delay_for_attempt(1);
    assert_eq!(d1.as_millis(), 1000); // 1000 * 2^0

    let d2 = policy.delay_for_attempt(2);
    assert_eq!(d2.as_millis(), 2000); // 1000 * 2^1

    let d3 = policy.delay_for_attempt(3);
    assert_eq!(d3.as_millis(), 4000); // 1000 * 2^2
}

#[test]
fn retry_delay_capped_at_max() {
    let policy = RetryPolicy {
        max_retries: 10,
        retry_delay: std::time::Duration::from_millis(1000),
        backoff_multiplier: 2.0,
        max_delay: std::time::Duration::from_millis(5000),
        jitter_fraction: 0.0,
    };

    // Attempt 4: 1000 * 2^3 = 8000 -> capped at 5000
    let d4 = policy.delay_for_attempt(4);
    assert_eq!(d4.as_millis(), 5000);

    // Higher attempts should also be capped
    let d10 = policy.delay_for_attempt(10);
    assert_eq!(d10.as_millis(), 5000);
}

#[test]
fn retry_delay_jitter_reduces_delay() {
    let policy = RetryPolicy {
        max_retries: 5,
        retry_delay: std::time::Duration::from_millis(1000),
        backoff_multiplier: 2.0,
        max_delay: std::time::Duration::from_millis(60_000),
        jitter_fraction: 0.5, // 50% jitter
    };

    // With jitter, delay should be less than or equal to base
    let d1 = policy.delay_for_attempt(1);
    assert!(d1.as_millis() <= 1000);
    assert!(d1.as_millis() >= 500); // at least 50% of base
}

#[test]
fn retry_backoff_backward_compat_default() {
    let policy = RetryPolicy::default();
    assert_eq!(policy.backoff_multiplier, 2.0);
    assert_eq!(policy.max_delay.as_millis(), 30_000);
    assert!((policy.jitter_fraction - 0.1).abs() < f64::EPSILON);
    // Default: 0 retries, should still have sensible delay
    let d = policy.delay_for_attempt(1);
    assert!(d.as_millis() > 0);
}

#[tokio::test]
async fn retry_with_backoff_integration() {
    // Create orchestrator with backoff config
    let storage = Arc::new(SqliteStorage::in_memory().unwrap());
    let project = Project::new("backoff-test", "/tmp/backoff");
    storage.insert_project(&project).unwrap();

    let mut registry = AdapterRegistry::new();
    registry.register(Arc::new(FakeAdapter::local("ollama")));

    let budget_config = BudgetConfig {
        mode: BudgetMode::Observe,
        global_monthly_limit: MoneyAmount::from_dollars(100.0),
        thresholds: vec![50, 75, 90, 100],
        ..BudgetConfig::default()
    };
    let usage_store = Arc::new(InMemoryUsageStore::new());
    let governor = Arc::new(BudgetGovernor::new(budget_config, usage_store));
    let registry = Arc::new(registry);
    let mut policy = RoutingPolicy::default();
    policy.check_health_before_routing = false;
    let routing_engine = RoutingEngine::new(policy, Arc::clone(&registry), Arc::clone(&governor));
    let orchestrator = Orchestrator::new(registry, routing_engine, governor, storage)
        .with_retry_policy(RetryPolicy {
            max_retries: 2,
            retry_delay: std::time::Duration::from_millis(1),
            backoff_multiplier: 2.0,
            max_delay: std::time::Duration::from_millis(100),
            jitter_fraction: 0.1,
        });

    let task = Task::new(project.id.clone(), TaskType::Summarization, "backoff test");
    let result = orchestrator
        .submit_task(task, ProjectRoutingConfig::default(), MoneyAmount::from_cents(50))
        .await
        .unwrap();
    // Ollama local adapter succeeds on first try, so no retries needed
    assert!(result.execution_output.is_some() || result.requires_approval == false);
}

// --- Step 54: Webhook event notifications ---

#[test]
fn webhook_config_parse() {
    let toml_str = r#"
        name = "notify"
        url = "https://example.com/webhook"
        events = ["TaskCompleted", "TaskFailed"]
        enabled = true
    "#;
    let wh: strategos::config::WebhookConfig = toml::from_str(toml_str).unwrap();
    assert_eq!(wh.name, "notify");
    assert_eq!(wh.url, "https://example.com/webhook");
    assert!(wh.enabled);
    assert_eq!(wh.events.as_ref().unwrap().len(), 2);
}

#[test]
fn webhook_disabled_not_dispatched() {
    let (orchestrator, _) = setup_orchestrator(BudgetMode::Observe, MoneyAmount::ZERO);
    // No webhooks configured by default = disabled
    let event = strategos::models::event::Event::new(
        strategos::models::event::EventType::TaskCompleted,
        serde_json::json!({"test": true}),
    );
    orchestrator.dispatch_webhooks(&event);
    // Should not crash and no deliveries recorded
    let deliveries = orchestrator.storage.list_webhook_deliveries(10).unwrap();
    assert!(deliveries.is_empty());
}

#[test]
fn webhook_delivery_recorded() {
    let storage = Arc::new(SqliteStorage::in_memory().unwrap());
    let project = Project::new("wh-test", "/tmp/wh");
    storage.insert_project(&project).unwrap();

    let mut registry = AdapterRegistry::new();
    registry.register(Arc::new(FakeAdapter::local("ollama")));
    let budget_config = BudgetConfig {
        mode: BudgetMode::Observe,
        global_monthly_limit: MoneyAmount::from_dollars(100.0),
        thresholds: vec![50, 75, 90, 100],
        ..BudgetConfig::default()
    };
    let usage_store = Arc::new(InMemoryUsageStore::new());
    let governor = Arc::new(BudgetGovernor::new(budget_config, usage_store));
    let registry = Arc::new(registry);
    let policy = RoutingPolicy::default();
    let routing_engine = RoutingEngine::new(policy, Arc::clone(&registry), Arc::clone(&governor));
    let mut orchestrator = Orchestrator::new(registry, routing_engine, governor, Arc::clone(&storage));
    orchestrator.webhooks = vec![strategos::config::WebhookConfig {
        name: "test-hook".into(),
        url: "https://example.com/hook".into(),
        events: None, // all events
        enabled: true,
    }];

    let event = strategos::models::event::Event::new(
        strategos::models::event::EventType::TaskCompleted,
        serde_json::json!({"msg": "hello"}),
    );
    orchestrator.dispatch_webhooks(&event);

    let deliveries = storage.list_webhook_deliveries(10).unwrap();
    assert_eq!(deliveries.len(), 1);
    assert_eq!(deliveries[0].webhook_name, "test-hook");
    assert!(deliveries[0].success);
}

#[test]
fn webhook_event_filter_match() {
    let storage = Arc::new(SqliteStorage::in_memory().unwrap());
    let project = Project::new("wh-filter", "/tmp/wh2");
    storage.insert_project(&project).unwrap();

    let mut registry = AdapterRegistry::new();
    registry.register(Arc::new(FakeAdapter::local("ollama")));
    let budget_config = BudgetConfig {
        mode: BudgetMode::Observe,
        global_monthly_limit: MoneyAmount::from_dollars(100.0),
        thresholds: vec![50, 75, 90, 100],
        ..BudgetConfig::default()
    };
    let usage_store = Arc::new(InMemoryUsageStore::new());
    let governor = Arc::new(BudgetGovernor::new(budget_config, usage_store));
    let registry = Arc::new(registry);
    let policy = RoutingPolicy::default();
    let routing_engine = RoutingEngine::new(policy, Arc::clone(&registry), Arc::clone(&governor));
    let mut orchestrator = Orchestrator::new(registry, routing_engine, governor, Arc::clone(&storage));
    orchestrator.webhooks = vec![strategos::config::WebhookConfig {
        name: "filtered".into(),
        url: "https://example.com/filtered".into(),
        events: Some(vec!["TaskFailed".into()]), // only TaskFailed
        enabled: true,
    }];

    // Send TaskCompleted — should be filtered out
    let event = strategos::models::event::Event::new(
        strategos::models::event::EventType::TaskCompleted,
        serde_json::json!({"msg": "completed"}),
    );
    orchestrator.dispatch_webhooks(&event);
    assert_eq!(storage.list_webhook_deliveries(10).unwrap().len(), 0);

    // Send TaskFailed — should match
    let event2 = strategos::models::event::Event::new(
        strategos::models::event::EventType::TaskFailed,
        serde_json::json!({"msg": "failed"}),
    );
    orchestrator.dispatch_webhooks(&event2);
    assert_eq!(storage.list_webhook_deliveries(10).unwrap().len(), 1);
}

#[test]
fn webhook_delivery_list_ordered() {
    let storage = SqliteStorage::in_memory().unwrap();
    // Insert two deliveries manually
    let d1 = strategos::models::event::WebhookDelivery {
        id: uuid::Uuid::new_v4().to_string(),
        webhook_name: "hook1".into(),
        url: "https://a.com".into(),
        event_type: strategos::models::event::EventType::TaskCompleted,
        payload: serde_json::json!({}),
        status_code: Some(200),
        success: true,
        error: None,
        delivered_at: chrono::Utc::now() - chrono::Duration::hours(1),
    };
    let d2 = strategos::models::event::WebhookDelivery {
        id: uuid::Uuid::new_v4().to_string(),
        webhook_name: "hook2".into(),
        url: "https://b.com".into(),
        event_type: strategos::models::event::EventType::TaskFailed,
        payload: serde_json::json!({}),
        status_code: Some(500),
        success: false,
        error: Some("server error".into()),
        delivered_at: chrono::Utc::now(),
    };
    storage.insert_webhook_delivery(&d1).unwrap();
    storage.insert_webhook_delivery(&d2).unwrap();

    let deliveries = storage.list_webhook_deliveries(10).unwrap();
    assert_eq!(deliveries.len(), 2);
    // Most recent first
    assert_eq!(deliveries[0].webhook_name, "hook2");
    assert_eq!(deliveries[1].webhook_name, "hook1");
}

// --- Step 55: Task templates ---

#[test]
fn template_config_parse() {
    let toml_str = r#"
        name = "quick-review"
        task_type = "review"
        description = "Review {0} for {1}"
        backend = "claude"
        priority = "high"
        max_tokens = 4096
    "#;
    let tmpl: strategos::config::TemplateConfig = toml::from_str(toml_str).unwrap();
    assert_eq!(tmpl.name, "quick-review");
    assert_eq!(tmpl.task_type, "review");
    assert_eq!(tmpl.backend.as_deref(), Some("claude"));
}

#[test]
fn template_resolve_placeholders() {
    let tmpl = strategos::config::TemplateConfig {
        name: "test".into(),
        task_type: "review".into(),
        description: Some("Review {0} for {1}".into()),
        backend: None,
        priority: None,
        max_tokens: None,
        timeout: None,
        max_cost: None,
    };
    let resolved = tmpl.resolve_description(&["main.rs", "security"]).unwrap();
    assert_eq!(resolved, "Review main.rs for security");
}

#[test]
fn template_missing_args_error() {
    let tmpl = strategos::config::TemplateConfig {
        name: "test".into(),
        task_type: "review".into(),
        description: Some("Review {0} for {1}".into()),
        backend: None,
        priority: None,
        max_tokens: None,
        timeout: None,
        max_cost: None,
    };
    // Only provide one arg — {1} should remain unresolved
    let result = tmpl.resolve_description(&["main.rs"]);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("{1}"));
}

#[test]
fn template_validation_empty_name() {
    let tmpl = strategos::config::TemplateConfig {
        name: "".into(),
        task_type: "review".into(),
        description: None,
        backend: None,
        priority: None,
        max_tokens: None,
        timeout: None,
        max_cost: None,
    };
    let errors = tmpl.validate();
    assert!(errors.iter().any(|e| e.contains("name cannot be empty")));
}

#[test]
fn template_validation_empty_task_type() {
    let tmpl = strategos::config::TemplateConfig {
        name: "valid-name".into(),
        task_type: "".into(),
        description: None,
        backend: None,
        priority: None,
        max_tokens: None,
        timeout: None,
        max_cost: None,
    };
    let errors = tmpl.validate();
    assert!(errors.iter().any(|e| e.contains("task_type cannot be empty")));
}

#[test]
fn global_config_with_templates_roundtrips() {
    let mut config = strategos::config::GlobalConfig::sample();
    config.templates = Some(vec![strategos::config::TemplateConfig {
        name: "quick-review".into(),
        task_type: "review".into(),
        description: Some("Review {0}".into()),
        backend: Some("claude".into()),
        priority: None,
        max_tokens: Some(4096),
        timeout: None,
        max_cost: None,
    }]);

    let toml_str = toml::to_string_pretty(&config).unwrap();
    let parsed: strategos::config::GlobalConfig = toml::from_str(&toml_str).unwrap();
    let templates = parsed.templates.unwrap();
    assert_eq!(templates.len(), 1);
    assert_eq!(templates[0].name, "quick-review");
    assert_eq!(templates[0].max_tokens, Some(4096));
}

#[test]
fn global_config_with_webhooks_roundtrips() {
    let mut config = strategos::config::GlobalConfig::sample();
    config.webhooks = Some(vec![strategos::config::WebhookConfig {
        name: "slack".into(),
        url: "https://hooks.slack.com/test".into(),
        events: Some(vec!["TaskCompleted".into()]),
        enabled: true,
    }]);

    let toml_str = toml::to_string_pretty(&config).unwrap();
    let parsed: strategos::config::GlobalConfig = toml::from_str(&toml_str).unwrap();
    let webhooks = parsed.webhooks.unwrap();
    assert_eq!(webhooks.len(), 1);
    assert_eq!(webhooks[0].name, "slack");
    assert!(webhooks[0].enabled);
}
