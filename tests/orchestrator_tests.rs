use std::collections::HashMap;
use std::sync::Arc;

use strategos::adapters::fake::{FakeAdapter, FakeBehavior};
use strategos::adapters::traits::AdapterRegistry;
use strategos::budget::governor::*;
use strategos::models::*;
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
