use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{BackendId, PrivacyLevel, ProjectId};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: ProjectId,
    pub name: String,
    pub path: PathBuf,
    pub default_backend: Option<BackendId>,
    pub fallback_chain: Vec<BackendId>,
    pub budget_limit_cents: Option<i64>,
    pub privacy: PrivacyLevel,
    pub tags: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Project {
    pub fn new(name: impl Into<String>, path: impl Into<PathBuf>) -> Self {
        let now = Utc::now();
        Self {
            id: ProjectId::new(),
            name: name.into(),
            path: path.into(),
            default_backend: None,
            fallback_chain: Vec::new(),
            budget_limit_cents: None,
            privacy: PrivacyLevel::default(),
            tags: Vec::new(),
            created_at: now,
            updated_at: now,
        }
    }
}
