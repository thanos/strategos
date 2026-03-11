use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::budget::governor::BudgetMode;
use crate::models::{BackendId, PrivacyLevel};

/// Top-level configuration loaded from TOML.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalConfig {
    pub default_backend: BackendId,
    pub monthly_budget_dollars: f64,
    pub budget_mode: BudgetMode,
    pub storage_path: Option<PathBuf>,
    pub backends: BackendsConfig,
    pub projects: Vec<ProjectConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendsConfig {
    pub claude: Option<ClaudeBackendConfig>,
    pub ollama: Option<OllamaBackendConfig>,
    pub opencode: Option<OpenCodeBackendConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeBackendConfig {
    pub api_key_env: String,
    pub model: String,
    pub monthly_budget_dollars: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaBackendConfig {
    pub endpoint: String,
    pub model: String,
    pub monthly_budget_dollars: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenCodeBackendConfig {
    pub binary_path: Option<String>,
    pub monthly_budget_dollars: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectConfig {
    pub name: String,
    pub path: PathBuf,
    pub default_backend: Option<BackendId>,
    pub fallback_chain: Option<Vec<BackendId>>,
    pub monthly_budget_dollars: Option<f64>,
    pub privacy: Option<PrivacyLevel>,
    pub tags: Option<Vec<String>>,
    pub task_overrides: Option<HashMap<String, BackendId>>,
}

impl GlobalConfig {
    pub fn sample() -> Self {
        Self {
            default_backend: BackendId::new("claude"),
            monthly_budget_dollars: 100.0,
            budget_mode: BudgetMode::Govern,
            storage_path: None,
            backends: BackendsConfig {
                claude: Some(ClaudeBackendConfig {
                    api_key_env: "ANTHROPIC_API_KEY".into(),
                    model: "claude-sonnet-4-20250514".into(),
                    monthly_budget_dollars: Some(80.0),
                }),
                ollama: Some(OllamaBackendConfig {
                    endpoint: "http://localhost:11434".into(),
                    model: "llama3".into(),
                    monthly_budget_dollars: None,
                }),
                opencode: None,
            },
            projects: vec![ProjectConfig {
                name: "my-project".into(),
                path: PathBuf::from("/home/user/projects/my-project"),
                default_backend: None,
                fallback_chain: None,
                monthly_budget_dollars: Some(20.0),
                privacy: Some(PrivacyLevel::Public),
                tags: Some(vec!["rust".into(), "backend".into()]),
                task_overrides: None,
            }],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_config_serializes_to_toml() {
        let config = GlobalConfig::sample();
        let toml_str = toml::to_string_pretty(&config).unwrap();
        assert!(toml_str.contains("claude"));
        assert!(toml_str.contains("monthly_budget_dollars"));
    }

    #[test]
    fn sample_config_roundtrips_through_toml() {
        let config = GlobalConfig::sample();
        let toml_str = toml::to_string_pretty(&config).unwrap();
        let parsed: GlobalConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.default_backend.as_str(), "claude");
        assert_eq!(parsed.monthly_budget_dollars, 100.0);
    }
}
