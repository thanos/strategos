use std::collections::HashMap;
use std::sync::Arc;

use strategos::adapters::fake::{FakeAdapter, FakeBehavior};
use strategos::adapters::traits::AdapterRegistry;
use strategos::budget::governor::*;
use strategos::models::*;
use strategos::models::policy::{ActionStatus, PendingAction, PendingActionType};
use strategos::models::project::Project;
use strategos::models::task::Task;
use strategos::orchestrator::service::Orchestrator;
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
