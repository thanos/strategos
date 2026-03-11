use std::path::Path;

use chrono::Utc;
use rusqlite::{Connection, params};

use crate::errors::StorageError;
use crate::models::project::Project;
use crate::models::{BackendId, MoneyAmount, ProjectId, PrivacyLevel};

use super::schema::SCHEMA_V1;

pub struct SqliteStorage {
    conn: Connection,
}

impl SqliteStorage {
    /// Open or create a database at the given path and apply migrations.
    pub fn open(path: &Path) -> Result<Self, StorageError> {
        let conn =
            Connection::open(path).map_err(|e| StorageError::Database(e.to_string()))?;
        let storage = Self { conn };
        storage.migrate()?;
        Ok(storage)
    }

    /// Create an in-memory database (for testing).
    pub fn in_memory() -> Result<Self, StorageError> {
        let conn =
            Connection::open_in_memory().map_err(|e| StorageError::Database(e.to_string()))?;
        let storage = Self { conn };
        storage.migrate()?;
        Ok(storage)
    }

    fn migrate(&self) -> Result<(), StorageError> {
        // Check current version
        let has_migrations = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='_migrations'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|e| StorageError::Database(e.to_string()))?;

        let current_version = if has_migrations > 0 {
            self.conn
                .query_row(
                    "SELECT COALESCE(MAX(version), 0) FROM _migrations",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap_or(0)
        } else {
            0
        };

        if current_version < 1 {
            self.conn
                .execute_batch(SCHEMA_V1)
                .map_err(|e| StorageError::Database(e.to_string()))?;
            self.conn
                .execute(
                    "INSERT INTO _migrations (version, applied_at) VALUES (?1, ?2)",
                    params![1, Utc::now().to_rfc3339()],
                )
                .map_err(|e| StorageError::Database(e.to_string()))?;
        }

        Ok(())
    }

    // -- Project CRUD -------------------------------------------------------

    pub fn insert_project(&self, project: &Project) -> Result<(), StorageError> {
        let tags_json =
            serde_json::to_string(&project.tags).map_err(|e| StorageError::Serialization(e.to_string()))?;
        let privacy_str = serde_json::to_string(&project.privacy)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;

        self.conn
            .execute(
                "INSERT INTO projects (id, name, path, privacy, tags, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    project.id.0.to_string(),
                    project.name,
                    project.path.to_string_lossy().to_string(),
                    privacy_str,
                    tags_json,
                    project.created_at.to_rfc3339(),
                    project.updated_at.to_rfc3339(),
                ],
            )
            .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(())
    }

    pub fn get_project(&self, id: &ProjectId) -> Result<Option<Project>, StorageError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, name, path, privacy, tags, created_at, updated_at FROM projects WHERE id = ?1",
            )
            .map_err(|e| StorageError::Database(e.to_string()))?;

        let result = stmt
            .query_row(params![id.0.to_string()], |row| {
                Ok(ProjectRow {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    path: row.get(2)?,
                    privacy: row.get(3)?,
                    tags: row.get(4)?,
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                })
            })
            .optional()
            .map_err(|e| StorageError::Database(e.to_string()))?;

        match result {
            Some(row) => Ok(Some(row.into_project()?)),
            None => Ok(None),
        }
    }

    pub fn list_projects(&self) -> Result<Vec<Project>, StorageError> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, name, path, privacy, tags, created_at, updated_at FROM projects")
            .map_err(|e| StorageError::Database(e.to_string()))?;

        let rows = stmt
            .query_map([], |row| {
                Ok(ProjectRow {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    path: row.get(2)?,
                    privacy: row.get(3)?,
                    tags: row.get(4)?,
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                })
            })
            .map_err(|e| StorageError::Database(e.to_string()))?;

        let mut projects = Vec::new();
        for row in rows {
            let row = row.map_err(|e| StorageError::Database(e.to_string()))?;
            projects.push(row.into_project()?);
        }
        Ok(projects)
    }

    pub fn delete_project(&self, id: &ProjectId) -> Result<(), StorageError> {
        let affected = self
            .conn
            .execute(
                "DELETE FROM projects WHERE id = ?1",
                params![id.0.to_string()],
            )
            .map_err(|e| StorageError::Database(e.to_string()))?;

        if affected == 0 {
            return Err(StorageError::NotFound(format!("project {}", id)));
        }
        Ok(())
    }

    // -- Usage queries ------------------------------------------------------

    pub fn insert_usage_record(
        &self,
        id: &str,
        task_id: &str,
        project_id: &str,
        backend_id: &str,
        input_tokens: u64,
        output_tokens: u64,
        cost_cents: i64,
        model: Option<&str>,
        recorded_at: &str,
    ) -> Result<(), StorageError> {
        self.conn
            .execute(
                "INSERT INTO usage_records (id, task_id, project_id, backend_id, input_tokens, output_tokens, cost_cents, model, recorded_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![id, task_id, project_id, backend_id, input_tokens as i64, output_tokens as i64, cost_cents, model, recorded_at],
            )
            .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(())
    }

    /// Total spend for the given month (YYYY-MM prefix match on recorded_at).
    pub fn total_spend_month(&self, year_month: &str) -> Result<MoneyAmount, StorageError> {
        let cents: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(SUM(cost_cents), 0) FROM usage_records WHERE recorded_at LIKE ?1",
                params![format!("{}%", year_month)],
                |row| row.get(0),
            )
            .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(MoneyAmount::from_cents(cents))
    }

    /// Spend for a specific backend in the given month.
    pub fn backend_spend_month(
        &self,
        backend_id: &BackendId,
        year_month: &str,
    ) -> Result<MoneyAmount, StorageError> {
        let cents: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(SUM(cost_cents), 0) FROM usage_records WHERE backend_id = ?1 AND recorded_at LIKE ?2",
                params![backend_id.as_str(), format!("{}%", year_month)],
                |row| row.get(0),
            )
            .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(MoneyAmount::from_cents(cents))
    }

    /// Spend for a specific project in the given month.
    pub fn project_spend_month(
        &self,
        project_id: &ProjectId,
        year_month: &str,
    ) -> Result<MoneyAmount, StorageError> {
        let cents: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(SUM(cost_cents), 0) FROM usage_records WHERE project_id = ?1 AND recorded_at LIKE ?2",
                params![project_id.0.to_string(), format!("{}%", year_month)],
                |row| row.get(0),
            )
            .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(MoneyAmount::from_cents(cents))
    }
}

// Internal helper for mapping rows
struct ProjectRow {
    id: String,
    name: String,
    path: String,
    privacy: String,
    tags: Option<String>,
    created_at: String,
    updated_at: String,
}

impl ProjectRow {
    fn into_project(self) -> Result<Project, StorageError> {
        let id = uuid::Uuid::parse_str(&self.id)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let privacy: PrivacyLevel = serde_json::from_str(&self.privacy)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let tags: Vec<String> = match self.tags {
            Some(ref s) => {
                serde_json::from_str(s).map_err(|e| StorageError::Serialization(e.to_string()))?
            }
            None => Vec::new(),
        };
        let created_at = chrono::DateTime::parse_from_rfc3339(&self.created_at)
            .map_err(|e| StorageError::Serialization(e.to_string()))?
            .with_timezone(&Utc);
        let updated_at = chrono::DateTime::parse_from_rfc3339(&self.updated_at)
            .map_err(|e| StorageError::Serialization(e.to_string()))?
            .with_timezone(&Utc);

        Ok(Project {
            id: ProjectId(id),
            name: self.name,
            path: self.path.into(),
            default_backend: None,
            fallback_chain: Vec::new(),
            budget_limit_cents: None,
            privacy,
            tags,
            created_at,
            updated_at,
        })
    }
}

// rusqlite helper
trait OptionalExt<T> {
    fn optional(self) -> Result<Option<T>, rusqlite::Error>;
}

impl<T> OptionalExt<T> for Result<T, rusqlite::Error> {
    fn optional(self) -> Result<Option<T>, rusqlite::Error> {
        match self {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::project::Project;

    #[test]
    fn create_in_memory_db() {
        let storage = SqliteStorage::in_memory().unwrap();
        let projects = storage.list_projects().unwrap();
        assert!(projects.is_empty());
    }

    #[test]
    fn project_crud_roundtrip() {
        let storage = SqliteStorage::in_memory().unwrap();

        let project = Project::new("test-project", "/tmp/test");
        storage.insert_project(&project).unwrap();

        let fetched = storage.get_project(&project.id).unwrap().unwrap();
        assert_eq!(fetched.name, "test-project");
        assert_eq!(fetched.privacy, PrivacyLevel::Public);

        let all = storage.list_projects().unwrap();
        assert_eq!(all.len(), 1);

        storage.delete_project(&project.id).unwrap();
        let all = storage.list_projects().unwrap();
        assert!(all.is_empty());
    }

    #[test]
    fn delete_nonexistent_project_returns_not_found() {
        let storage = SqliteStorage::in_memory().unwrap();
        let result = storage.delete_project(&ProjectId::new());
        assert!(matches!(result, Err(StorageError::NotFound(_))));
    }

    #[test]
    fn usage_spend_queries() {
        let storage = SqliteStorage::in_memory().unwrap();

        // Insert a project and a task first (for foreign keys)
        let project = Project::new("proj", "/tmp/p");
        storage.insert_project(&project).unwrap();

        // Insert task (bypass full model, just need a row)
        storage.conn.execute(
            "INSERT INTO tasks (id, project_id, task_type, description, priority, status, created_at, updated_at)
             VALUES ('task-1', ?1, 'Planning', 'test', 'Normal', 'Pending', '2026-03-01T00:00:00Z', '2026-03-01T00:00:00Z')",
            params![project.id.0.to_string()],
        ).unwrap();

        // Insert usage records
        storage.insert_usage_record(
            "u1", "task-1", &project.id.0.to_string(), "claude",
            100, 50, 500, Some("claude-sonnet"), "2026-03-15T10:00:00Z",
        ).unwrap();
        storage.insert_usage_record(
            "u2", "task-1", &project.id.0.to_string(), "claude",
            200, 100, 300, Some("claude-sonnet"), "2026-03-16T10:00:00Z",
        ).unwrap();
        storage.insert_usage_record(
            "u3", "task-1", &project.id.0.to_string(), "ollama",
            50, 25, 0, Some("llama3"), "2026-03-16T11:00:00Z",
        ).unwrap();

        // Total March spend
        let total = storage.total_spend_month("2026-03").unwrap();
        assert_eq!(total.cents, 800);

        // Claude spend
        let claude_spend = storage.backend_spend_month(&BackendId::new("claude"), "2026-03").unwrap();
        assert_eq!(claude_spend.cents, 800);

        // Ollama spend
        let ollama_spend = storage.backend_spend_month(&BackendId::new("ollama"), "2026-03").unwrap();
        assert_eq!(ollama_spend.cents, 0);

        // Project spend
        let proj_spend = storage.project_spend_month(&project.id, "2026-03").unwrap();
        assert_eq!(proj_spend.cents, 800);

        // Different month returns zero
        let feb = storage.total_spend_month("2026-02").unwrap();
        assert_eq!(feb.cents, 0);
    }
}
