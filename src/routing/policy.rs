use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::models::{BackendId, TaskType};

/// Routing policy loaded from TOML configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingPolicy {
    /// Default backend for each task type.
    pub task_defaults: HashMap<TaskType, BackendId>,
    /// Global fallback chain when the selected backend is unavailable.
    pub global_fallback_chain: Vec<BackendId>,
    /// When a backend is over budget, downgrade to this backend.
    pub budget_downgrade_map: HashMap<BackendId, BackendId>,
    /// Whether to check backend health before routing (default: true).
    pub check_health_before_routing: bool,
}

impl Default for RoutingPolicy {
    fn default() -> Self {
        let claude = BackendId::new("claude");
        let ollama = BackendId::new("ollama");
        let opencode = BackendId::new("opencode");

        let mut task_defaults = HashMap::new();
        task_defaults.insert(TaskType::DeepCodeReasoning, claude.clone());
        task_defaults.insert(TaskType::Planning, claude.clone());
        task_defaults.insert(TaskType::Review, claude.clone());
        task_defaults.insert(TaskType::CommitPreparation, claude.clone());
        task_defaults.insert(TaskType::Summarization, ollama.clone());
        task_defaults.insert(TaskType::BacklogTriage, ollama.clone());
        task_defaults.insert(TaskType::LowCostDrafting, ollama.clone());
        task_defaults.insert(TaskType::PrivateLocalTask, ollama.clone());
        task_defaults.insert(TaskType::Experimental, opencode.clone());

        let mut budget_downgrade_map = HashMap::new();
        budget_downgrade_map.insert(claude.clone(), ollama.clone());
        budget_downgrade_map.insert(opencode, ollama.clone());

        Self {
            task_defaults,
            global_fallback_chain: vec![claude, ollama],
            budget_downgrade_map,
            check_health_before_routing: true,
        }
    }
}
