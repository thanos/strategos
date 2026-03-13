use std::collections::HashMap;
use std::path::{Path, PathBuf};

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
    pub log_level: Option<String>,
    pub fallback_chain: Option<Vec<BackendId>>,
    pub retry_policy: Option<RetryPolicyConfig>,
    pub backends: BackendsConfig,
    pub projects: Vec<ProjectConfig>,
    pub webhooks: Option<Vec<WebhookConfig>>,
    pub templates: Option<Vec<TemplateConfig>>,
}

/// Configuration for automatic retry of transient failures.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryPolicyConfig {
    /// Maximum number of retry attempts (default: 2).
    pub max_retries: u32,
    /// Delay between retries in milliseconds (default: 1000).
    pub retry_delay_ms: u64,
    /// Multiplier for exponential backoff (default: 2.0).
    #[serde(default = "default_backoff_multiplier")]
    pub backoff_multiplier: f64,
    /// Maximum delay in milliseconds (default: 30000).
    #[serde(default = "default_max_delay_ms")]
    pub max_delay_ms: u64,
    /// Jitter fraction 0.0–1.0 applied to delay (default: 0.1).
    #[serde(default = "default_jitter_fraction")]
    pub jitter_fraction: f64,
}

fn default_backoff_multiplier() -> f64 {
    2.0
}

fn default_max_delay_ms() -> u64 {
    30_000
}

fn default_jitter_fraction() -> f64 {
    0.1
}

/// Configuration for a webhook endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookConfig {
    pub name: String,
    pub url: String,
    /// Optional list of event types to filter on. If empty/None, all events are sent.
    pub events: Option<Vec<String>>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

/// Configuration for a task template.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateConfig {
    pub name: String,
    pub task_type: String,
    pub description: Option<String>,
    pub backend: Option<String>,
    pub priority: Option<String>,
    pub max_tokens: Option<u64>,
    pub timeout: Option<u64>,
    pub max_cost: Option<i64>,
}

impl TemplateConfig {
    /// Resolve the template description with placeholder substitution.
    /// Placeholders are `{0}`, `{1}`, etc. replaced by positional args.
    pub fn resolve_description(&self, args: &[&str]) -> Result<String, String> {
        let base = self.description.as_deref().unwrap_or("");
        let mut result = base.to_string();
        for (i, arg) in args.iter().enumerate() {
            let placeholder = format!("{{{}}}", i);
            result = result.replace(&placeholder, arg);
        }
        // Check for unresolved placeholders
        if let Some(pos) = result.find('{') {
            if let Some(end) = result[pos..].find('}') {
                let placeholder = &result[pos..pos + end + 1];
                return Err(format!("unresolved placeholder: {}", placeholder));
            }
        }
        Ok(result)
    }

    /// Validate the template config.
    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();
        if self.name.is_empty() {
            errors.push("template name cannot be empty".into());
        }
        if self.task_type.is_empty() {
            errors.push(format!("template '{}': task_type cannot be empty", self.name));
        }
        errors
    }
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
    /// Returns the default config file path for the current platform.
    pub fn default_path() -> PathBuf {
        if let Some(config_dir) = dirs_config() {
            config_dir.join("strategos").join("config.toml")
        } else {
            PathBuf::from("strategos.toml")
        }
    }

    /// Returns the default storage (database) path.
    pub fn default_storage_path() -> PathBuf {
        if let Some(data_dir) = dirs_data() {
            data_dir.join("strategos").join("strategos.db")
        } else {
            PathBuf::from("strategos.db")
        }
    }

    /// Load config from a TOML file.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: Self = toml::from_str(&content)?;
        Ok(config)
    }

    /// Load from default path, or return the sample config if file doesn't exist.
    pub fn load_or_default() -> Self {
        let path = Self::default_path();
        if path.exists() {
            match Self::load(&path) {
                Ok(config) => config,
                Err(e) => {
                    eprintln!("warning: failed to load config from {}: {}", path.display(), e);
                    Self::sample()
                }
            }
        } else {
            Self::sample()
        }
    }

    /// Write this config to a TOML file.
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }

    /// Returns the storage path, using the configured value or default.
    pub fn storage_path(&self) -> PathBuf {
        self.storage_path
            .clone()
            .unwrap_or_else(Self::default_storage_path)
    }

    /// Look up a project config by name.
    pub fn find_project(&self, name: &str) -> Option<&ProjectConfig> {
        self.projects.iter().find(|p| p.name == name)
    }

    /// Returns the list of configured backend names.
    pub fn configured_backends(&self) -> Vec<&str> {
        let mut backends = Vec::new();
        if self.backends.claude.is_some() {
            backends.push("claude");
        }
        if self.backends.ollama.is_some() {
            backends.push("ollama");
        }
        if self.backends.opencode.is_some() {
            backends.push("opencode");
        }
        backends
    }

    /// Validate config and return errors if any.
    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();
        let known_backends = self.configured_backends();

        // default_backend must reference a configured backend
        if !known_backends.contains(&self.default_backend.as_str()) {
            errors.push(format!(
                "default_backend '{}' is not a configured backend (available: {})",
                self.default_backend,
                known_backends.join(", ")
            ));
        }

        // Budget amounts must be non-negative
        if self.monthly_budget_dollars < 0.0 {
            errors.push("monthly_budget_dollars must be non-negative".into());
        }

        // Global fallback chain must reference configured backends
        if let Some(ref chain) = self.fallback_chain {
            for backend in chain {
                if !known_backends.contains(&backend.as_str()) {
                    errors.push(format!(
                        "fallback_chain references unknown backend '{}'",
                        backend
                    ));
                }
            }
        }

        // Per-project validation
        let mut project_names = std::collections::HashSet::new();
        for project in &self.projects {
            if !project_names.insert(&project.name) {
                errors.push(format!("duplicate project name '{}'", project.name));
            }

            if let Some(ref backend) = project.default_backend {
                if !known_backends.contains(&backend.as_str()) {
                    errors.push(format!(
                        "project '{}': default_backend '{}' is not a configured backend",
                        project.name, backend
                    ));
                }
            }

            if let Some(ref chain) = project.fallback_chain {
                for backend in chain {
                    if !known_backends.contains(&backend.as_str()) {
                        errors.push(format!(
                            "project '{}': fallback_chain references unknown backend '{}'",
                            project.name, backend
                        ));
                    }
                }
            }

            if let Some(budget) = project.monthly_budget_dollars {
                if budget < 0.0 {
                    errors.push(format!(
                        "project '{}': monthly_budget_dollars must be non-negative",
                        project.name
                    ));
                }
            }

            if let Some(ref overrides) = project.task_overrides {
                let valid_task_types = [
                    "deep-code-reasoning", "planning", "review", "commit-preparation",
                    "summarization", "backlog-triage", "low-cost-drafting", "private-local",
                    "experimental",
                ];
                for (key, backend) in overrides {
                    if !valid_task_types.contains(&key.as_str()) {
                        errors.push(format!(
                            "project '{}': unknown task type '{}' in task_overrides",
                            project.name, key
                        ));
                    }
                    if !known_backends.contains(&backend.as_str()) {
                        errors.push(format!(
                            "project '{}': task_overrides '{}' references unknown backend '{}'",
                            project.name, key, backend
                        ));
                    }
                }
            }
        }

        errors
    }

    pub fn sample() -> Self {
        Self {
            default_backend: BackendId::new("claude"),
            monthly_budget_dollars: 100.0,
            budget_mode: BudgetMode::Govern,
            storage_path: None,
            log_level: Some("info".into()),
            retry_policy: None,
            fallback_chain: Some(vec![
                BackendId::new("claude"),
                BackendId::new("ollama"),
            ]),
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
            projects: vec![
                ProjectConfig {
                    name: "my-project".into(),
                    path: PathBuf::from("/home/user/projects/my-project"),
                    default_backend: None,
                    fallback_chain: None,
                    monthly_budget_dollars: Some(20.0),
                    privacy: Some(PrivacyLevel::Public),
                    tags: Some(vec!["rust".into(), "backend".into()]),
                    task_overrides: Some({
                        let mut m = HashMap::new();
                        m.insert("summarization".into(), BackendId::new("ollama"));
                        m
                    }),
                },
                ProjectConfig {
                    name: "private-research".into(),
                    path: PathBuf::from("/home/user/projects/private-research"),
                    default_backend: Some(BackendId::new("ollama")),
                    fallback_chain: Some(vec![BackendId::new("ollama")]),
                    monthly_budget_dollars: Some(5.0),
                    privacy: Some(PrivacyLevel::LocalOnly),
                    tags: Some(vec!["research".into()]),
                    task_overrides: None,
                },
            ],
            webhooks: None,
            templates: None,
        }
    }
}

// Platform-specific directory helpers (minimal, no extra crate dependency)
fn dirs_config() -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        std::env::var("HOME")
            .ok()
            .map(|h| PathBuf::from(h).join(".config"))
    } else if cfg!(target_os = "linux") {
        std::env::var("XDG_CONFIG_HOME")
            .ok()
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var("HOME")
                    .ok()
                    .map(|h| PathBuf::from(h).join(".config"))
            })
    } else {
        std::env::var("HOME")
            .ok()
            .map(|h| PathBuf::from(h).join(".config"))
    }
}

fn dirs_data() -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        std::env::var("HOME")
            .ok()
            .map(|h| PathBuf::from(h).join(".local").join("share"))
    } else if cfg!(target_os = "linux") {
        std::env::var("XDG_DATA_HOME")
            .ok()
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var("HOME")
                    .ok()
                    .map(|h| PathBuf::from(h).join(".local").join("share"))
            })
    } else {
        std::env::var("HOME")
            .ok()
            .map(|h| PathBuf::from(h).join(".local").join("share"))
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

    #[test]
    fn save_and_load_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");

        let config = GlobalConfig::sample();
        config.save(&path).unwrap();

        let loaded = GlobalConfig::load(&path).unwrap();
        assert_eq!(loaded.default_backend.as_str(), "claude");
        assert_eq!(loaded.budget_mode, BudgetMode::Govern);
    }

    #[test]
    fn default_paths_are_not_empty() {
        let config_path = GlobalConfig::default_path();
        assert!(config_path.to_string_lossy().contains("strategos"));

        let storage_path = GlobalConfig::default_storage_path();
        assert!(storage_path.to_string_lossy().contains("strategos"));
    }

    #[test]
    fn sample_config_validates_ok() {
        let config = GlobalConfig::sample();
        let errors = config.validate();
        assert!(errors.is_empty(), "sample config should validate: {:?}", errors);
    }

    #[test]
    fn validate_unknown_default_backend() {
        let mut config = GlobalConfig::sample();
        config.default_backend = BackendId::new("nonexistent");
        let errors = config.validate();
        assert!(errors.iter().any(|e| e.contains("default_backend") && e.contains("nonexistent")));
    }

    #[test]
    fn validate_negative_budget() {
        let mut config = GlobalConfig::sample();
        config.monthly_budget_dollars = -10.0;
        let errors = config.validate();
        assert!(errors.iter().any(|e| e.contains("non-negative")));
    }

    #[test]
    fn validate_duplicate_project_names() {
        let mut config = GlobalConfig::sample();
        config.projects.push(ProjectConfig {
            name: "my-project".into(),
            path: PathBuf::from("/tmp/dup"),
            default_backend: None,
            fallback_chain: None,
            monthly_budget_dollars: None,
            privacy: None,
            tags: None,
            task_overrides: None,
        });
        let errors = config.validate();
        assert!(errors.iter().any(|e| e.contains("duplicate project name")));
    }

    #[test]
    fn validate_unknown_backend_in_fallback_chain() {
        let mut config = GlobalConfig::sample();
        config.fallback_chain = Some(vec![
            BackendId::new("claude"),
            BackendId::new("mystery"),
        ]);
        let errors = config.validate();
        assert!(errors.iter().any(|e| e.contains("fallback_chain") && e.contains("mystery")));
    }

    #[test]
    fn validate_unknown_task_type_in_overrides() {
        let mut config = GlobalConfig::sample();
        let mut overrides = HashMap::new();
        overrides.insert("not-a-task".into(), BackendId::new("claude"));
        config.projects[0].task_overrides = Some(overrides);
        let errors = config.validate();
        assert!(errors.iter().any(|e| e.contains("unknown task type") && e.contains("not-a-task")));
    }

    #[test]
    fn validate_project_backend_references() {
        let mut config = GlobalConfig::sample();
        config.projects[0].default_backend = Some(BackendId::new("ghost"));
        let errors = config.validate();
        assert!(errors.iter().any(|e| e.contains("ghost") && e.contains("not a configured backend")));
    }

    #[test]
    fn find_project_by_name() {
        let config = GlobalConfig::sample();
        assert!(config.find_project("my-project").is_some());
        assert!(config.find_project("private-research").is_some());
        assert!(config.find_project("nonexistent").is_none());
    }

    #[test]
    fn sample_config_includes_log_level_and_fallback() {
        let config = GlobalConfig::sample();
        assert_eq!(config.log_level.as_deref(), Some("info"));
        assert!(config.fallback_chain.is_some());
        let chain = config.fallback_chain.unwrap();
        assert!(chain.len() >= 2);
    }

    #[test]
    fn sample_config_includes_task_overrides() {
        let config = GlobalConfig::sample();
        let pc = config.find_project("my-project").unwrap();
        assert!(pc.task_overrides.is_some());
        let overrides = pc.task_overrides.as_ref().unwrap();
        assert!(overrides.contains_key("summarization"));
    }
}
