use std::collections::HashMap;
use std::sync::Arc;

use strategos::budget::governor::*;
use strategos::models::{BackendId, MoneyAmount, ProjectId, TaskId};

fn claude() -> BackendId {
    BackendId::new("claude")
}

fn ollama() -> BackendId {
    BackendId::new("ollama")
}

fn test_project() -> ProjectId {
    ProjectId::new()
}

fn eval_request(backend: &BackendId, project: &ProjectId) -> BudgetEvaluationRequest {
    BudgetEvaluationRequest {
        task_id: TaskId::new(),
        project_id: project.clone(),
        backend_id: backend.clone(),
        estimated_cost: MoneyAmount::from_cents(100),
    }
}

fn config_with_mode(mode: BudgetMode) -> BudgetConfig {
    let mut downgrade_map = HashMap::new();
    downgrade_map.insert(claude(), ollama());

    BudgetConfig {
        mode,
        global_monthly_limit: MoneyAmount::from_dollars(100.0),
        backend_limits: HashMap::new(),
        project_limits: HashMap::new(),
        thresholds: vec![50, 75, 90, 100],
        downgrade_map,
    }
}

// ---------------------------------------------------------------------------
// Observe mode: never intervenes
// ---------------------------------------------------------------------------

#[tokio::test]
async fn observe_mode_allows_at_any_threshold() {
    let store = Arc::new(
        InMemoryUsageStore::new().with_global(MoneyAmount::from_dollars(95.0)),
    );
    let governor = BudgetGovernor::new(config_with_mode(BudgetMode::Observe), store);

    let project = test_project();
    let decision = governor.evaluate(&eval_request(&claude(), &project)).await.unwrap();
    assert_eq!(decision.action, BudgetAction::Allow);
}

#[tokio::test]
async fn observe_mode_allows_even_at_100() {
    let store = Arc::new(
        InMemoryUsageStore::new().with_global(MoneyAmount::from_dollars(100.0)),
    );
    let governor = BudgetGovernor::new(config_with_mode(BudgetMode::Observe), store);

    let project = test_project();
    let decision = governor.evaluate(&eval_request(&claude(), &project)).await.unwrap();
    assert_eq!(decision.action, BudgetAction::Allow);
}

// ---------------------------------------------------------------------------
// Warn mode: warns but never blocks
// ---------------------------------------------------------------------------

#[tokio::test]
async fn warn_mode_allows_under_50() {
    let store = Arc::new(
        InMemoryUsageStore::new().with_global(MoneyAmount::from_dollars(30.0)),
    );
    let governor = BudgetGovernor::new(config_with_mode(BudgetMode::Warn), store);

    let project = test_project();
    let decision = governor.evaluate(&eval_request(&claude(), &project)).await.unwrap();
    assert_eq!(decision.action, BudgetAction::Allow);
}

#[tokio::test]
async fn warn_mode_warns_at_75() {
    let store = Arc::new(
        InMemoryUsageStore::new().with_global(MoneyAmount::from_dollars(75.0)),
    );
    let governor = BudgetGovernor::new(config_with_mode(BudgetMode::Warn), store);

    let project = test_project();
    let decision = governor.evaluate(&eval_request(&claude(), &project)).await.unwrap();
    assert_eq!(decision.action, BudgetAction::Warn);
    assert!(!decision.warnings.is_empty());
}

#[tokio::test]
async fn warn_mode_warns_at_100_never_blocks() {
    let store = Arc::new(
        InMemoryUsageStore::new().with_global(MoneyAmount::from_dollars(100.0)),
    );
    let governor = BudgetGovernor::new(config_with_mode(BudgetMode::Warn), store);

    let project = test_project();
    let decision = governor.evaluate(&eval_request(&claude(), &project)).await.unwrap();
    assert_eq!(decision.action, BudgetAction::Warn);
}

// ---------------------------------------------------------------------------
// Govern mode: downgrades, approvals, blocks
// ---------------------------------------------------------------------------

#[tokio::test]
async fn govern_mode_allows_under_50() {
    let store = Arc::new(
        InMemoryUsageStore::new().with_global(MoneyAmount::from_dollars(40.0)),
    );
    let governor = BudgetGovernor::new(config_with_mode(BudgetMode::Govern), store);

    let project = test_project();
    let decision = governor.evaluate(&eval_request(&claude(), &project)).await.unwrap();
    assert_eq!(decision.action, BudgetAction::Allow);
}

#[tokio::test]
async fn govern_mode_warns_at_50() {
    let store = Arc::new(
        InMemoryUsageStore::new().with_global(MoneyAmount::from_dollars(50.0)),
    );
    let governor = BudgetGovernor::new(config_with_mode(BudgetMode::Govern), store);

    let project = test_project();
    let decision = governor.evaluate(&eval_request(&claude(), &project)).await.unwrap();
    assert_eq!(decision.action, BudgetAction::Warn);
}

#[tokio::test]
async fn govern_mode_downgrades_at_75() {
    let store = Arc::new(
        InMemoryUsageStore::new().with_global(MoneyAmount::from_dollars(75.0)),
    );
    let governor = BudgetGovernor::new(config_with_mode(BudgetMode::Govern), store);

    let project = test_project();
    let decision = governor.evaluate(&eval_request(&claude(), &project)).await.unwrap();
    assert_eq!(decision.action, BudgetAction::DowngradeTo(ollama()));
}

#[tokio::test]
async fn govern_mode_requires_approval_at_90() {
    let store = Arc::new(
        InMemoryUsageStore::new().with_global(MoneyAmount::from_dollars(90.0)),
    );
    let governor = BudgetGovernor::new(config_with_mode(BudgetMode::Govern), store);

    let project = test_project();
    let decision = governor.evaluate(&eval_request(&claude(), &project)).await.unwrap();
    assert_eq!(decision.action, BudgetAction::RequireApproval);
}

#[tokio::test]
async fn govern_mode_blocks_at_100() {
    let store = Arc::new(
        InMemoryUsageStore::new().with_global(MoneyAmount::from_dollars(100.0)),
    );
    let governor = BudgetGovernor::new(config_with_mode(BudgetMode::Govern), store);

    let project = test_project();
    let decision = governor.evaluate(&eval_request(&claude(), &project)).await.unwrap();
    assert_eq!(decision.action, BudgetAction::Block);
}

// ---------------------------------------------------------------------------
// Enforce mode: stricter than Govern
// ---------------------------------------------------------------------------

#[tokio::test]
async fn enforce_mode_blocks_at_90() {
    let store = Arc::new(
        InMemoryUsageStore::new().with_global(MoneyAmount::from_dollars(90.0)),
    );
    let governor = BudgetGovernor::new(config_with_mode(BudgetMode::Enforce), store);

    let project = test_project();
    let decision = governor.evaluate(&eval_request(&claude(), &project)).await.unwrap();
    assert_eq!(decision.action, BudgetAction::Block);
}

#[tokio::test]
async fn enforce_mode_downgrades_at_75() {
    let store = Arc::new(
        InMemoryUsageStore::new().with_global(MoneyAmount::from_dollars(75.0)),
    );
    let governor = BudgetGovernor::new(config_with_mode(BudgetMode::Enforce), store);

    let project = test_project();
    let decision = governor.evaluate(&eval_request(&claude(), &project)).await.unwrap();
    assert_eq!(decision.action, BudgetAction::DowngradeTo(ollama()));
}

// ---------------------------------------------------------------------------
// Multi-scope: most restrictive wins
// ---------------------------------------------------------------------------

#[tokio::test]
async fn most_restrictive_scope_wins() {
    // Global at 40% (allow), but backend at 90% (require approval in Govern)
    let mut config = config_with_mode(BudgetMode::Govern);
    config.backend_limits.insert(claude(), MoneyAmount::from_dollars(50.0));

    let store = Arc::new(
        InMemoryUsageStore::new()
            .with_global(MoneyAmount::from_dollars(40.0))
            .with_backend(claude(), MoneyAmount::from_dollars(45.0)), // 90% of $50
    );
    let governor = BudgetGovernor::new(config, store);

    let project = test_project();
    let decision = governor.evaluate(&eval_request(&claude(), &project)).await.unwrap();
    assert_eq!(decision.action, BudgetAction::RequireApproval);
    assert_eq!(decision.scope_states.len(), 2); // global + backend
}

#[tokio::test]
async fn project_scope_can_be_most_restrictive() {
    let project = test_project();
    let mut config = config_with_mode(BudgetMode::Govern);
    config.project_limits.insert(project.clone(), MoneyAmount::from_dollars(10.0));

    let store = Arc::new(
        InMemoryUsageStore::new()
            .with_global(MoneyAmount::from_dollars(20.0))     // 20% of $100
            .with_project(project.clone(), MoneyAmount::from_dollars(10.0)), // 100% of $10
    );
    let governor = BudgetGovernor::new(config, store);

    let decision = governor.evaluate(&eval_request(&claude(), &project)).await.unwrap();
    assert_eq!(decision.action, BudgetAction::Block);
}

// ---------------------------------------------------------------------------
// Scope states are always returned
// ---------------------------------------------------------------------------

#[tokio::test]
async fn scope_states_include_all_applicable_scopes() {
    let project = test_project();
    let mut config = config_with_mode(BudgetMode::Warn);
    config.backend_limits.insert(claude(), MoneyAmount::from_dollars(80.0));
    config.project_limits.insert(project.clone(), MoneyAmount::from_dollars(20.0));

    let store = Arc::new(InMemoryUsageStore::new());
    let governor = BudgetGovernor::new(config, store);

    let decision = governor.evaluate(&eval_request(&claude(), &project)).await.unwrap();
    assert_eq!(decision.scope_states.len(), 3); // global + backend + project
}
