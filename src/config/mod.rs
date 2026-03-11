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
}
