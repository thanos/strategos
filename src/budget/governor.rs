use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::errors::BudgetError;
use crate::models::{BackendId, MoneyAmount, ProjectId, TaskId};

use super::thresholds::DEFAULT_THRESHOLDS;

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BudgetMode {
    Observe,
    Warn,
    Govern,
    Enforce,
}

impl Default for BudgetMode {
    fn default() -> Self {
        Self::Warn
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetConfig {
    pub mode: BudgetMode,
    pub global_monthly_limit: MoneyAmount,
    pub backend_limits: HashMap<BackendId, MoneyAmount>,
    pub project_limits: HashMap<ProjectId, MoneyAmount>,
    pub thresholds: Vec<u8>,
    pub downgrade_map: HashMap<BackendId, BackendId>,
}

impl Default for BudgetConfig {
    fn default() -> Self {
        Self {
            mode: BudgetMode::Warn,
            global_monthly_limit: MoneyAmount::from_dollars(100.0),
            backend_limits: HashMap::new(),
            project_limits: HashMap::new(),
            thresholds: DEFAULT_THRESHOLDS.to_vec(),
            downgrade_map: HashMap::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Request / Decision types
// ---------------------------------------------------------------------------

pub struct BudgetEvaluationRequest {
    pub task_id: TaskId,
    pub project_id: ProjectId,
    pub backend_id: BackendId,
    pub estimated_cost: MoneyAmount,
}

#[derive(Debug, Clone)]
pub struct BudgetDecision {
    pub action: BudgetAction,
    pub reason: BudgetReason,
    pub scope_states: Vec<BudgetScopeState>,
    pub warnings: Vec<BudgetWarning>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BudgetAction {
    Allow,
    Warn,
    DowngradeTo(BackendId),
    RequireApproval,
    Block,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BudgetReason {
    WithinBudget,
    ThresholdExceeded {
        scope: BudgetScope,
        threshold_pct: u8,
        current_pct: u8,
    },
    OverBudget {
        scope: BudgetScope,
    },
    ModeAllows,
}

impl fmt::Display for BudgetReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BudgetReason::WithinBudget => write!(f, "within budget"),
            BudgetReason::ThresholdExceeded {
                scope,
                threshold_pct,
                current_pct,
            } => write!(
                f,
                "{scope:?} at {current_pct}% (threshold {threshold_pct}%)"
            ),
            BudgetReason::OverBudget { scope } => write!(f, "{scope:?} over budget"),
            BudgetReason::ModeAllows => write!(f, "mode allows"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct BudgetScopeState {
    pub scope: BudgetScope,
    pub limit: MoneyAmount,
    pub spent: MoneyAmount,
    pub percentage: u8,
    pub threshold_breached: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum BudgetScope {
    Global,
    Backend(BackendId),
    Project(ProjectId),
}

#[derive(Debug, Clone)]
pub struct BudgetWarning {
    pub scope: BudgetScope,
    pub message: String,
    pub threshold_pct: u8,
}

// ---------------------------------------------------------------------------
// Usage store trait
// ---------------------------------------------------------------------------

#[async_trait]
pub trait UsageStore: Send + Sync {
    async fn total_spend_current_month(&self) -> Result<MoneyAmount, BudgetError>;
    async fn backend_spend_current_month(
        &self,
        backend: &BackendId,
    ) -> Result<MoneyAmount, BudgetError>;
    async fn project_spend_current_month(
        &self,
        project: &ProjectId,
    ) -> Result<MoneyAmount, BudgetError>;
}

// ---------------------------------------------------------------------------
// In-memory usage store for testing
// ---------------------------------------------------------------------------

pub struct InMemoryUsageStore {
    pub global_spend: MoneyAmount,
    pub backend_spend: HashMap<BackendId, MoneyAmount>,
    pub project_spend: HashMap<ProjectId, MoneyAmount>,
}

impl InMemoryUsageStore {
    pub fn new() -> Self {
        Self {
            global_spend: MoneyAmount::ZERO,
            backend_spend: HashMap::new(),
            project_spend: HashMap::new(),
        }
    }

    pub fn with_global(mut self, amount: MoneyAmount) -> Self {
        self.global_spend = amount;
        self
    }

    pub fn with_backend(mut self, backend: BackendId, amount: MoneyAmount) -> Self {
        self.backend_spend.insert(backend, amount);
        self
    }

    pub fn with_project(mut self, project: ProjectId, amount: MoneyAmount) -> Self {
        self.project_spend.insert(project, amount);
        self
    }
}

#[async_trait]
impl UsageStore for InMemoryUsageStore {
    async fn total_spend_current_month(&self) -> Result<MoneyAmount, BudgetError> {
        Ok(self.global_spend)
    }

    async fn backend_spend_current_month(
        &self,
        backend: &BackendId,
    ) -> Result<MoneyAmount, BudgetError> {
        Ok(self
            .backend_spend
            .get(backend)
            .copied()
            .unwrap_or(MoneyAmount::ZERO))
    }

    async fn project_spend_current_month(
        &self,
        project: &ProjectId,
    ) -> Result<MoneyAmount, BudgetError> {
        Ok(self
            .project_spend
            .get(project)
            .copied()
            .unwrap_or(MoneyAmount::ZERO))
    }
}

// ---------------------------------------------------------------------------
// Budget Governor
// ---------------------------------------------------------------------------

pub struct BudgetGovernor {
    config: BudgetConfig,
    usage_store: Arc<dyn UsageStore>,
}

impl BudgetGovernor {
    pub fn new(config: BudgetConfig, usage_store: Arc<dyn UsageStore>) -> Self {
        Self {
            config,
            usage_store,
        }
    }

    pub async fn evaluate(
        &self,
        request: &BudgetEvaluationRequest,
    ) -> Result<BudgetDecision, BudgetError> {
        let mut scope_states = Vec::new();
        let mut warnings = Vec::new();
        let mut worst_action = BudgetAction::Allow;
        let mut worst_reason = BudgetReason::WithinBudget;

        // Evaluate global scope
        let global_spent = self.usage_store.total_spend_current_month().await?;
        let global_state = self.evaluate_scope(
            BudgetScope::Global,
            global_spent,
            self.config.global_monthly_limit,
            &request.backend_id,
            &mut warnings,
        );
        if action_severity(&global_state.action) > action_severity(&worst_action) {
            worst_action = global_state.action.clone();
            worst_reason = global_state.reason.clone();
        }
        scope_states.push(global_state.state);

        // Evaluate backend scope
        if let Some(&limit) = self.config.backend_limits.get(&request.backend_id) {
            let backend_spent = self
                .usage_store
                .backend_spend_current_month(&request.backend_id)
                .await?;
            let backend_state = self.evaluate_scope(
                BudgetScope::Backend(request.backend_id.clone()),
                backend_spent,
                limit,
                &request.backend_id,
                &mut warnings,
            );
            if action_severity(&backend_state.action) > action_severity(&worst_action) {
                worst_action = backend_state.action.clone();
                worst_reason = backend_state.reason.clone();
            }
            scope_states.push(backend_state.state);
        }

        // Evaluate project scope
        if let Some(&limit) = self.config.project_limits.get(&request.project_id) {
            let project_spent = self
                .usage_store
                .project_spend_current_month(&request.project_id)
                .await?;
            let project_state = self.evaluate_scope(
                BudgetScope::Project(request.project_id.clone()),
                project_spent,
                limit,
                &request.backend_id,
                &mut warnings,
            );
            if action_severity(&project_state.action) > action_severity(&worst_action) {
                worst_action = project_state.action.clone();
                worst_reason = project_state.reason.clone();
            }
            scope_states.push(project_state.state);
        }

        Ok(BudgetDecision {
            action: worst_action,
            reason: worst_reason,
            scope_states,
            warnings,
        })
    }

    fn evaluate_scope(
        &self,
        scope: BudgetScope,
        spent: MoneyAmount,
        limit: MoneyAmount,
        backend_id: &BackendId,
        warnings: &mut Vec<BudgetWarning>,
    ) -> ScopeEvaluation {
        let pct = spent.percentage_of(limit);

        // Find the highest breached threshold
        let breached_threshold = self
            .config
            .thresholds
            .iter()
            .rev()
            .find(|&&t| pct >= t)
            .copied();

        let state = BudgetScopeState {
            scope: scope.clone(),
            limit,
            spent,
            percentage: pct,
            threshold_breached: breached_threshold,
        };

        let (action, reason) = match breached_threshold {
            None => (BudgetAction::Allow, BudgetReason::WithinBudget),
            Some(threshold) => {
                let base_reason = BudgetReason::ThresholdExceeded {
                    scope: scope.clone(),
                    threshold_pct: threshold,
                    current_pct: pct,
                };

                let action = match self.config.mode {
                    BudgetMode::Observe => {
                        // Observe mode: never intervene
                        BudgetAction::Allow
                    }
                    BudgetMode::Warn => {
                        // Warn mode: warn but never block
                        warnings.push(BudgetWarning {
                            scope: scope.clone(),
                            message: format!(
                                "{:?} at {}% of budget (threshold {}%)",
                                scope, pct, threshold
                            ),
                            threshold_pct: threshold,
                        });
                        BudgetAction::Warn
                    }
                    BudgetMode::Govern => match threshold {
                        t if t >= 100 => BudgetAction::Block,
                        t if t >= 90 => BudgetAction::RequireApproval,
                        t if t >= 75 => {
                            if let Some(downgrade_to) =
                                self.config.downgrade_map.get(backend_id)
                            {
                                BudgetAction::DowngradeTo(downgrade_to.clone())
                            } else {
                                BudgetAction::Warn
                            }
                        }
                        _ => {
                            warnings.push(BudgetWarning {
                                scope: scope.clone(),
                                message: format!("{:?} at {}%", scope, pct),
                                threshold_pct: threshold,
                            });
                            BudgetAction::Warn
                        }
                    },
                    BudgetMode::Enforce => match threshold {
                        t if t >= 100 => BudgetAction::Block,
                        t if t >= 90 => BudgetAction::Block,
                        t if t >= 75 => {
                            if let Some(downgrade_to) =
                                self.config.downgrade_map.get(backend_id)
                            {
                                BudgetAction::DowngradeTo(downgrade_to.clone())
                            } else {
                                BudgetAction::Block
                            }
                        }
                        _ => {
                            if let Some(downgrade_to) =
                                self.config.downgrade_map.get(backend_id)
                            {
                                warnings.push(BudgetWarning {
                                    scope: scope.clone(),
                                    message: format!("{:?} at {}%, suggesting downgrade", scope, pct),
                                    threshold_pct: threshold,
                                });
                                BudgetAction::DowngradeTo(downgrade_to.clone())
                            } else {
                                BudgetAction::Warn
                            }
                        }
                    },
                };

                let final_reason = if self.config.mode == BudgetMode::Observe {
                    BudgetReason::ModeAllows
                } else {
                    base_reason
                };

                (action, final_reason)
            }
        };

        ScopeEvaluation {
            state,
            action,
            reason,
        }
    }
}

struct ScopeEvaluation {
    state: BudgetScopeState,
    action: BudgetAction,
    reason: BudgetReason,
}

fn action_severity(action: &BudgetAction) -> u8 {
    match action {
        BudgetAction::Allow => 0,
        BudgetAction::Warn => 1,
        BudgetAction::DowngradeTo(_) => 2,
        BudgetAction::RequireApproval => 3,
        BudgetAction::Block => 4,
    }
}
