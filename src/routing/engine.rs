use std::collections::HashMap;
use std::sync::Arc;

use tracing::instrument;

use crate::adapters::traits::AdapterRegistry;
use crate::budget::governor::{BudgetAction, BudgetEvaluationRequest, BudgetGovernor};
use crate::errors::{BackendEvaluation, RoutingError};
use crate::models::{BackendId, MoneyAmount, PrivacyLevel, ProjectId, TaskId, TaskType};

use super::policy::RoutingPolicy;

/// The routing engine selects an execution backend for each task.
/// Every decision is explicit, explainable, and testable.
pub struct RoutingEngine {
    policy: RoutingPolicy,
    registry: Arc<AdapterRegistry>,
    budget_governor: Arc<BudgetGovernor>,
}

/// Input to the routing engine.
pub struct RoutingRequest {
    pub task_id: TaskId,
    pub task_type: TaskType,
    pub project_id: ProjectId,
    pub project_config: ProjectRoutingConfig,
    pub backend_override: Option<BackendId>,
    pub estimated_cost: MoneyAmount,
}

/// Per-project routing configuration.
pub struct ProjectRoutingConfig {
    pub default_backend: Option<BackendId>,
    pub fallback_chain: Vec<BackendId>,
    pub privacy: PrivacyLevel,
    pub task_overrides: HashMap<TaskType, BackendId>,
}

impl Default for ProjectRoutingConfig {
    fn default() -> Self {
        Self {
            default_backend: None,
            fallback_chain: Vec::new(),
            privacy: PrivacyLevel::Public,
            task_overrides: HashMap::new(),
        }
    }
}

/// The result of a routing decision, with full explanation.
#[derive(Debug)]
pub struct RoutingDecision {
    pub selected_backend: BackendId,
    pub reason: RoutingReason,
    pub fallback_applied: bool,
    pub budget_downgrade_applied: bool,
    pub evaluated_backends: Vec<BackendEvaluation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoutingReason {
    UserOverride,
    ProjectDefault,
    ProjectTaskOverride,
    TaskTypeDefault,
    BudgetDowngrade { original: BackendId },
    FallbackChain { failed: Vec<BackendId> },
    PrivacyConstraint,
    OnlyEligibleBackend,
}

impl RoutingEngine {
    pub fn new(
        policy: RoutingPolicy,
        registry: Arc<AdapterRegistry>,
        budget_governor: Arc<BudgetGovernor>,
    ) -> Self {
        Self {
            policy,
            registry,
            budget_governor,
        }
    }

    #[instrument(skip(self, request), fields(task_type = ?request.task_type, project_id = %request.project_id))]
    pub async fn route(&self, request: RoutingRequest) -> Result<RoutingDecision, RoutingError> {
        let mut evaluations = Vec::new();

        // Step 1: User override
        if let Some(ref override_id) = request.backend_override {
            if let Some(decision) =
                self.try_backend(override_id, &request, RoutingReason::UserOverride, &mut evaluations).await?
            {
                return Ok(decision);
            }
        }

        // Step 2: Project task-type override
        if let Some(backend_id) = request.project_config.task_overrides.get(&request.task_type) {
            if let Some(decision) =
                self.try_backend(backend_id, &request, RoutingReason::ProjectTaskOverride, &mut evaluations).await?
            {
                return Ok(decision);
            }
        }

        // Step 3: Project default
        if let Some(ref backend_id) = request.project_config.default_backend {
            if let Some(decision) =
                self.try_backend(backend_id, &request, RoutingReason::ProjectDefault, &mut evaluations).await?
            {
                return Ok(decision);
            }
        }

        // Step 4: Task type default from policy
        if let Some(backend_id) = self.policy.task_defaults.get(&request.task_type) {
            if let Some(decision) =
                self.try_backend(backend_id, &request, RoutingReason::TaskTypeDefault, &mut evaluations).await?
            {
                return Ok(decision);
            }

            // Step 5: Budget downgrade
            if let Some(downgrade_id) = self.policy.budget_downgrade_map.get(backend_id) {
                if let Some(decision) = self
                    .try_backend(
                        downgrade_id,
                        &request,
                        RoutingReason::BudgetDowngrade {
                            original: backend_id.clone(),
                        },
                        &mut evaluations,
                    )
                    .await?
                {
                    return Ok(RoutingDecision {
                        budget_downgrade_applied: true,
                        ..decision
                    });
                }
            }
        }

        // Step 6: Fallback chain (project-specific, then global)
        let fallback_chain = if !request.project_config.fallback_chain.is_empty() {
            &request.project_config.fallback_chain
        } else {
            &self.policy.global_fallback_chain
        };

        let mut failed_fallbacks = Vec::new();
        for backend_id in fallback_chain {
            // Skip backends already evaluated
            if evaluations.iter().any(|e| &e.backend_id == backend_id) {
                failed_fallbacks.push(backend_id.clone());
                continue;
            }
            if let Some(decision) = self
                .try_backend(
                    backend_id,
                    &request,
                    RoutingReason::FallbackChain {
                        failed: failed_fallbacks.clone(),
                    },
                    &mut evaluations,
                )
                .await?
            {
                return Ok(RoutingDecision {
                    fallback_applied: true,
                    ..decision
                });
            }
            failed_fallbacks.push(backend_id.clone());
        }

        Err(RoutingError::AllFallbacksFailed(evaluations))
    }

    /// Try a specific backend. Returns Some(decision) if the backend is eligible and allowed.
    /// Returns None if the backend is ineligible (and records the evaluation).
    /// Returns Err only if budget evaluation produces a hard block.
    async fn try_backend(
        &self,
        backend_id: &BackendId,
        request: &RoutingRequest,
        reason: RoutingReason,
        evaluations: &mut Vec<BackendEvaluation>,
    ) -> Result<Option<RoutingDecision>, RoutingError> {
        // Check availability
        let adapter = match self.registry.get(backend_id) {
            Some(a) => a,
            None => {
                evaluations.push(BackendEvaluation {
                    backend_id: backend_id.clone(),
                    eligible: false,
                    rejection_reason: Some("backend not registered".into()),
                });
                return Ok(None);
            }
        };

        // Check privacy constraint
        if request.project_config.privacy == PrivacyLevel::LocalOnly
            && !adapter.capabilities().local_execution
        {
            evaluations.push(BackendEvaluation {
                backend_id: backend_id.clone(),
                eligible: false,
                rejection_reason: Some("project requires local execution".into()),
            });
            return Ok(None);
        }

        // Check capability
        if !adapter.capabilities().supports_task_type(&request.task_type) {
            evaluations.push(BackendEvaluation {
                backend_id: backend_id.clone(),
                eligible: false,
                rejection_reason: Some(format!(
                    "lacks required capabilities for {:?}",
                    request.task_type
                )),
            });
            return Ok(None);
        }

        // Check budget
        let budget_request = BudgetEvaluationRequest {
            task_id: request.task_id.clone(),
            project_id: request.project_id.clone(),
            backend_id: backend_id.clone(),
            estimated_cost: request.estimated_cost,
        };

        let budget_decision = self
            .budget_governor
            .evaluate(&budget_request)
            .await
            .map_err(|e| RoutingError::BudgetBlocked(e.to_string()))?;

        match budget_decision.action {
            BudgetAction::Block => {
                evaluations.push(BackendEvaluation {
                    backend_id: backend_id.clone(),
                    eligible: false,
                    rejection_reason: Some(format!("budget blocked: {}", budget_decision.reason)),
                });
                return Ok(None);
            }
            BudgetAction::RequireApproval => {
                evaluations.push(BackendEvaluation {
                    backend_id: backend_id.clone(),
                    eligible: false,
                    rejection_reason: Some("requires budget approval".into()),
                });
                return Ok(None);
            }
            BudgetAction::DowngradeTo(_) => {
                // Budget suggests downgrade — mark this backend ineligible and let
                // the caller try the downgrade target
                evaluations.push(BackendEvaluation {
                    backend_id: backend_id.clone(),
                    eligible: false,
                    rejection_reason: Some("budget recommends downgrade".into()),
                });
                return Ok(None);
            }
            BudgetAction::Allow | BudgetAction::Warn => {
                // Proceed
            }
        }

        evaluations.push(BackendEvaluation {
            backend_id: backend_id.clone(),
            eligible: true,
            rejection_reason: None,
        });

        let is_downgrade = matches!(reason, RoutingReason::BudgetDowngrade { .. });

        Ok(Some(RoutingDecision {
            selected_backend: backend_id.clone(),
            reason,
            fallback_applied: false,
            budget_downgrade_applied: is_downgrade,
            evaluated_backends: evaluations.clone(),
        }))
    }
}
