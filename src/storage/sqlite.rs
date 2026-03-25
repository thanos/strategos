use std::path::Path;

use async_trait::async_trait;
use chrono::Utc;
use rusqlite::{Connection, params};

use crate::budget::governor::UsageStore;
use crate::errors::BudgetError;
use crate::errors::StorageError;
use crate::models::event::{Event, EventType};
use crate::models::policy::{ActionStatus, PendingAction, PendingActionType};
use crate::models::project::Project;
use crate::models::task::{Task, TaskStatus};
use crate::models::usage::UsageRecord;
use crate::models::{
    ActionId, BackendId, EventId, MoneyAmount, Priority, PrivacyLevel, ProjectId, TaskId, TaskType,
};

use super::schema::{SCHEMA_V1, SCHEMA_V2, SCHEMA_V3, SCHEMA_V4, SCHEMA_V5, SCHEMA_V6, SCHEMA_V7, SCHEMA_V8};

pub struct SqliteStorage {
    conn: Connection,
}

impl SqliteStorage {
    /// Open or create a database at the given path and apply migrations.
    pub fn open(path: &Path) -> Result<Self, StorageError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| StorageError::Database(format!("cannot create directory: {}", e)))?;
        }
        let conn =
            Connection::open(path).map_err(|e| StorageError::Database(e.to_string()))?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
            .map_err(|e| StorageError::Database(e.to_string()))?;
        let storage = Self { conn };
        storage.migrate()?;
        Ok(storage)
    }

    /// Create an in-memory database (for testing).
    pub fn in_memory() -> Result<Self, StorageError> {
        let conn =
            Connection::open_in_memory().map_err(|e| StorageError::Database(e.to_string()))?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")
            .map_err(|e| StorageError::Database(e.to_string()))?;
        let storage = Self { conn };
        storage.migrate()?;
        Ok(storage)
    }

    /// Expose connection reference for testing.
    pub fn conn_ref(&self) -> &Connection {
        &self.conn
    }

    fn migrate(&self) -> Result<(), StorageError> {
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

        if current_version < 2 {
            self.conn
                .execute_batch(SCHEMA_V2)
                .map_err(|e| StorageError::Database(e.to_string()))?;
            self.conn
                .execute(
                    "INSERT INTO _migrations (version, applied_at) VALUES (?1, ?2)",
                    params![2, Utc::now().to_rfc3339()],
                )
                .map_err(|e| StorageError::Database(e.to_string()))?;
        }

        if current_version < 3 {
            self.conn
                .execute_batch(SCHEMA_V3)
                .map_err(|e| StorageError::Database(e.to_string()))?;
            self.conn
                .execute(
                    "INSERT INTO _migrations (version, applied_at) VALUES (?1, ?2)",
                    params![3, Utc::now().to_rfc3339()],
                )
                .map_err(|e| StorageError::Database(e.to_string()))?;
        }

        if current_version < 4 {
            self.conn
                .execute_batch(SCHEMA_V4)
                .map_err(|e| StorageError::Database(e.to_string()))?;
            self.conn
                .execute(
                    "INSERT INTO _migrations (version, applied_at) VALUES (?1, ?2)",
                    params![4, Utc::now().to_rfc3339()],
                )
                .map_err(|e| StorageError::Database(e.to_string()))?;
        }

        if current_version < 5 {
            self.conn
                .execute_batch(SCHEMA_V5)
                .map_err(|e| StorageError::Database(e.to_string()))?;
            self.conn
                .execute(
                    "INSERT INTO _migrations (version, applied_at) VALUES (?1, ?2)",
                    params![5, Utc::now().to_rfc3339()],
                )
                .map_err(|e| StorageError::Database(e.to_string()))?;
        }

        if current_version < 6 {
            self.conn
                .execute_batch(SCHEMA_V6)
                .map_err(|e| StorageError::Database(e.to_string()))?;
            self.conn
                .execute(
                    "INSERT INTO _migrations (version, applied_at) VALUES (?1, ?2)",
                    params![6, Utc::now().to_rfc3339()],
                )
                .map_err(|e| StorageError::Database(e.to_string()))?;
        }

        if current_version < 7 {
            self.conn
                .execute_batch(SCHEMA_V7)
                .map_err(|e| StorageError::Database(e.to_string()))?;
            self.conn
                .execute(
                    "INSERT INTO _migrations (version, applied_at) VALUES (?1, ?2)",
                    params![7, Utc::now().to_rfc3339()],
                )
                .map_err(|e| StorageError::Database(e.to_string()))?;
        }

        if current_version < 8 {
            self.conn
                .execute_batch(SCHEMA_V8)
                .map_err(|e| StorageError::Database(e.to_string()))?;
            self.conn
                .execute(
                    "INSERT INTO _migrations (version, applied_at) VALUES (?1, ?2)",
                    params![8, Utc::now().to_rfc3339()],
                )
                .map_err(|e| StorageError::Database(e.to_string()))?;
        }

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Project CRUD
    // -----------------------------------------------------------------------

    pub fn insert_project(&self, project: &Project) -> Result<(), StorageError> {
        let tags_json = serde_json::to_string(&project.tags)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
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

    pub fn update_project(&self, project: &Project) -> Result<(), StorageError> {
        let tags_json = serde_json::to_string(&project.tags)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let privacy_str = serde_json::to_string(&project.privacy)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;

        self.conn
            .execute(
                "UPDATE projects SET name = ?1, path = ?2, privacy = ?3, tags = ?4, updated_at = ?5 WHERE id = ?6",
                params![
                    project.name,
                    project.path.to_string_lossy().to_string(),
                    privacy_str,
                    tags_json,
                    chrono::Utc::now().to_rfc3339(),
                    project.id.0.to_string(),
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

    pub fn get_project_by_name(&self, name: &str) -> Result<Option<Project>, StorageError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, name, path, privacy, tags, created_at, updated_at FROM projects WHERE name = ?1",
            )
            .map_err(|e| StorageError::Database(e.to_string()))?;

        let result = stmt
            .query_row(params![name], |row| {
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
            .prepare("SELECT id, name, path, privacy, tags, created_at, updated_at FROM projects ORDER BY name")
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

    // -----------------------------------------------------------------------
    // Task CRUD
    // -----------------------------------------------------------------------

    pub fn insert_task(&self, task: &Task) -> Result<(), StorageError> {
        let task_type_str = serde_json::to_string(&task.task_type)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let priority_str = serde_json::to_string(&task.priority)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let status_str = serde_json::to_string(&task.status)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let override_str = task.backend_override.as_ref().map(|b| b.as_str().to_string());
        let queued_at_str = task.queued_at.map(|dt| dt.to_rfc3339());
        let tags_json = if task.tags.is_empty() {
            None
        } else {
            Some(serde_json::to_string(&task.tags)
                .map_err(|e| StorageError::Serialization(e.to_string()))?)
        };

        self.conn
            .execute(
                "INSERT INTO tasks (id, project_id, task_type, description, priority, status, backend_override, created_at, updated_at, queued_at, tags)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    task.id.0.to_string(),
                    task.project_id.0.to_string(),
                    task_type_str,
                    task.description,
                    priority_str,
                    status_str,
                    override_str,
                    task.created_at.to_rfc3339(),
                    task.updated_at.to_rfc3339(),
                    queued_at_str,
                    tags_json,
                ],
            )
            .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(())
    }

    pub fn get_task(&self, id: &TaskId) -> Result<Option<Task>, StorageError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, project_id, task_type, description, priority, status, backend_override, created_at, updated_at, queued_at, tags
                 FROM tasks WHERE id = ?1",
            )
            .map_err(|e| StorageError::Database(e.to_string()))?;

        let result = stmt
            .query_row(params![id.0.to_string()], |row| {
                Ok(TaskRow {
                    id: row.get(0)?,
                    project_id: row.get(1)?,
                    task_type: row.get(2)?,
                    description: row.get(3)?,
                    priority: row.get(4)?,
                    status: row.get(5)?,
                    backend_override: row.get(6)?,
                    created_at: row.get(7)?,
                    updated_at: row.get(8)?,
                    queued_at: row.get(9)?,
                    tags: row.get(10)?,
                })
            })
            .optional()
            .map_err(|e| StorageError::Database(e.to_string()))?;

        match result {
            Some(row) => Ok(Some(row.into_task()?)),
            None => Ok(None),
        }
    }

    pub fn list_tasks_by_project(&self, project_id: &ProjectId) -> Result<Vec<Task>, StorageError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, project_id, task_type, description, priority, status, backend_override, created_at, updated_at, queued_at, tags
                 FROM tasks WHERE project_id = ?1 ORDER BY created_at DESC",
            )
            .map_err(|e| StorageError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![project_id.0.to_string()], |row| {
                Ok(TaskRow {
                    id: row.get(0)?,
                    project_id: row.get(1)?,
                    task_type: row.get(2)?,
                    description: row.get(3)?,
                    priority: row.get(4)?,
                    status: row.get(5)?,
                    backend_override: row.get(6)?,
                    created_at: row.get(7)?,
                    updated_at: row.get(8)?,
                    queued_at: row.get(9)?,
                    tags: row.get(10)?,
                })
            })
            .map_err(|e| StorageError::Database(e.to_string()))?;

        let mut tasks = Vec::new();
        for row in rows {
            let row = row.map_err(|e| StorageError::Database(e.to_string()))?;
            tasks.push(row.into_task()?);
        }
        Ok(tasks)
    }

    pub fn update_task_status(
        &self,
        id: &TaskId,
        status: TaskStatus,
    ) -> Result<(), StorageError> {
        let status_str = serde_json::to_string(&status)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let affected = self
            .conn
            .execute(
                "UPDATE tasks SET status = ?1, updated_at = ?2 WHERE id = ?3",
                params![status_str, Utc::now().to_rfc3339(), id.0.to_string()],
            )
            .map_err(|e| StorageError::Database(e.to_string()))?;

        if affected == 0 {
            return Err(StorageError::NotFound(format!("task {}", id)));
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Usage records
    // -----------------------------------------------------------------------

    pub fn insert_usage(&self, record: &UsageRecord) -> Result<(), StorageError> {
        self.conn
            .execute(
                "INSERT INTO usage_records (id, task_id, project_id, backend_id, input_tokens, output_tokens, cost_cents, model, recorded_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    record.id.0.to_string(),
                    record.task_id.0.to_string(),
                    record.project_id.0.to_string(),
                    record.backend_id.as_str(),
                    record.input_tokens as i64,
                    record.output_tokens as i64,
                    record.cost.cents,
                    record.model,
                    record.recorded_at.to_rfc3339(),
                ],
            )
            .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(())
    }

    /// Keep the old string-based insert for backwards compatibility with existing tests.
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

    /// List usage records with optional filters.
    pub fn list_usage_records(
        &self,
        project_id: Option<&ProjectId>,
        backend_id: Option<&BackendId>,
        since: Option<&str>,
        limit: usize,
    ) -> Result<Vec<UsageRecord>, StorageError> {
        let mut sql = String::from(
            "SELECT id, task_id, project_id, backend_id, input_tokens, output_tokens, cost_cents, model, recorded_at
             FROM usage_records WHERE 1=1"
        );
        let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(pid) = project_id {
            params_vec.push(Box::new(pid.0.to_string()));
            sql.push_str(&format!(" AND project_id = ?{}", params_vec.len()));
        }
        if let Some(bid) = backend_id {
            params_vec.push(Box::new(bid.as_str().to_string()));
            sql.push_str(&format!(" AND backend_id = ?{}", params_vec.len()));
        }
        if let Some(since_date) = since {
            params_vec.push(Box::new(since_date.to_string()));
            sql.push_str(&format!(" AND recorded_at >= ?{}", params_vec.len()));
        }
        params_vec.push(Box::new(limit as i64));
        sql.push_str(&format!(" ORDER BY recorded_at DESC LIMIT ?{}", params_vec.len()));

        let mut stmt = self
            .conn
            .prepare(&sql)
            .map_err(|e| StorageError::Database(e.to_string()))?;

        let param_refs: Vec<&dyn rusqlite::types::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();

        let rows = stmt
            .query_map(param_refs.as_slice(), |row| {
                Ok(UsageRecordRow {
                    id: row.get(0)?,
                    task_id: row.get(1)?,
                    project_id: row.get(2)?,
                    backend_id: row.get(3)?,
                    input_tokens: row.get(4)?,
                    output_tokens: row.get(5)?,
                    cost_cents: row.get(6)?,
                    model: row.get(7)?,
                    recorded_at: row.get(8)?,
                })
            })
            .map_err(|e| StorageError::Database(e.to_string()))?;

        let mut records = Vec::new();
        for row in rows {
            let r = row.map_err(|e| StorageError::Database(e.to_string()))?;
            records.push(r.into_usage_record()?);
        }
        Ok(records)
    }

    // -----------------------------------------------------------------------
    // Events
    // -----------------------------------------------------------------------

    pub fn insert_event(&self, event: &Event) -> Result<(), StorageError> {
        let event_type_str = serde_json::to_string(&event.event_type)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let payload_str = serde_json::to_string(&event.payload)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;

        self.conn
            .execute(
                "INSERT INTO events (id, event_type, project_id, task_id, payload, timestamp)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    event.id.0.to_string(),
                    event_type_str,
                    event.project_id.as_ref().map(|p| p.0.to_string()),
                    event.task_id.as_ref().map(|t| t.0.to_string()),
                    payload_str,
                    event.timestamp.to_rfc3339(),
                ],
            )
            .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(())
    }

    pub fn list_events_recent(&self, limit: usize) -> Result<Vec<Event>, StorageError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, event_type, project_id, task_id, payload, timestamp
                 FROM events ORDER BY timestamp DESC LIMIT ?1",
            )
            .map_err(|e| StorageError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![limit as i64], |row| {
                Ok(EventRow {
                    id: row.get(0)?,
                    event_type: row.get(1)?,
                    project_id: row.get(2)?,
                    task_id: row.get(3)?,
                    payload: row.get(4)?,
                    timestamp: row.get(5)?,
                })
            })
            .map_err(|e| StorageError::Database(e.to_string()))?;

        let mut events = Vec::new();
        for row in rows {
            let row = row.map_err(|e| StorageError::Database(e.to_string()))?;
            events.push(row.into_event()?);
        }
        Ok(events)
    }

    pub fn list_events_by_project(
        &self,
        project_id: &ProjectId,
        limit: usize,
    ) -> Result<Vec<Event>, StorageError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, event_type, project_id, task_id, payload, timestamp
                 FROM events WHERE project_id = ?1 ORDER BY timestamp DESC LIMIT ?2",
            )
            .map_err(|e| StorageError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![project_id.0.to_string(), limit as i64], |row| {
                Ok(EventRow {
                    id: row.get(0)?,
                    event_type: row.get(1)?,
                    project_id: row.get(2)?,
                    task_id: row.get(3)?,
                    payload: row.get(4)?,
                    timestamp: row.get(5)?,
                })
            })
            .map_err(|e| StorageError::Database(e.to_string()))?;

        let mut events = Vec::new();
        for row in rows {
            let row = row.map_err(|e| StorageError::Database(e.to_string()))?;
            events.push(row.into_event()?);
        }
        Ok(events)
    }

    // -----------------------------------------------------------------------
    // Pending actions
    // -----------------------------------------------------------------------

    pub fn insert_pending_action(&self, action: &PendingAction) -> Result<(), StorageError> {
        let action_type_str = serde_json::to_string(&action.action_type)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let status_str = serde_json::to_string(&action.status)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let payload_str = serde_json::to_string(&action.payload)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;

        self.conn
            .execute(
                "INSERT INTO pending_actions (id, action_type, project_id, task_id, description, payload, status, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    action.id.0.to_string(),
                    action_type_str,
                    action.project_id.0.to_string(),
                    action.task_id.as_ref().map(|t| t.0.to_string()),
                    action.description,
                    payload_str,
                    status_str,
                    action.created_at.to_rfc3339(),
                ],
            )
            .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(())
    }

    pub fn get_pending_action(&self, id: &ActionId) -> Result<Option<PendingAction>, StorageError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, action_type, project_id, task_id, description, payload, status, created_at
                 FROM pending_actions WHERE id = ?1",
            )
            .map_err(|e| StorageError::Database(e.to_string()))?;

        let result = stmt
            .query_row(params![id.0.to_string()], |row| {
                Ok(PendingActionRow {
                    id: row.get(0)?,
                    action_type: row.get(1)?,
                    project_id: row.get(2)?,
                    task_id: row.get(3)?,
                    description: row.get(4)?,
                    payload: row.get(5)?,
                    status: row.get(6)?,
                    created_at: row.get(7)?,
                })
            })
            .optional()
            .map_err(|e| StorageError::Database(e.to_string()))?;

        match result {
            Some(row) => Ok(Some(row.into_pending_action()?)),
            None => Ok(None),
        }
    }

    pub fn list_pending_actions(&self) -> Result<Vec<PendingAction>, StorageError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, action_type, project_id, task_id, description, payload, status, created_at
                 FROM pending_actions WHERE status = '\"Pending\"' ORDER BY created_at",
            )
            .map_err(|e| StorageError::Database(e.to_string()))?;

        let rows = stmt
            .query_map([], |row| {
                Ok(PendingActionRow {
                    id: row.get(0)?,
                    action_type: row.get(1)?,
                    project_id: row.get(2)?,
                    task_id: row.get(3)?,
                    description: row.get(4)?,
                    payload: row.get(5)?,
                    status: row.get(6)?,
                    created_at: row.get(7)?,
                })
            })
            .map_err(|e| StorageError::Database(e.to_string()))?;

        let mut actions = Vec::new();
        for row in rows {
            let row = row.map_err(|e| StorageError::Database(e.to_string()))?;
            actions.push(row.into_pending_action()?);
        }
        Ok(actions)
    }

    pub fn list_all_actions(&self, limit: usize) -> Result<Vec<PendingAction>, StorageError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, action_type, project_id, task_id, description, payload, status, created_at
                 FROM pending_actions ORDER BY created_at DESC LIMIT ?1",
            )
            .map_err(|e| StorageError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![limit as i64], |row| {
                Ok(PendingActionRow {
                    id: row.get(0)?,
                    action_type: row.get(1)?,
                    project_id: row.get(2)?,
                    task_id: row.get(3)?,
                    description: row.get(4)?,
                    payload: row.get(5)?,
                    status: row.get(6)?,
                    created_at: row.get(7)?,
                })
            })
            .map_err(|e| StorageError::Database(e.to_string()))?;

        let mut actions = Vec::new();
        for row in rows {
            let row = row.map_err(|e| StorageError::Database(e.to_string()))?;
            actions.push(row.into_pending_action()?);
        }
        Ok(actions)
    }

    pub fn update_action_status(
        &self,
        id: &ActionId,
        status: ActionStatus,
    ) -> Result<(), StorageError> {
        let status_str = serde_json::to_string(&status)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let affected = self
            .conn
            .execute(
                "UPDATE pending_actions SET status = ?1 WHERE id = ?2",
                params![status_str, id.0.to_string()],
            )
            .map_err(|e| StorageError::Database(e.to_string()))?;

        if affected == 0 {
            return Err(StorageError::NotFound(format!("action {}", id.0)));
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Routing history
    // -----------------------------------------------------------------------

    pub fn insert_routing_history(
        &self,
        task_id: &TaskId,
        selected_backend: &str,
        reason: &str,
        fallback_applied: bool,
        budget_downgrade_applied: bool,
    ) -> Result<(), StorageError> {
        let id = uuid::Uuid::new_v4().to_string();
        self.conn
            .execute(
                "INSERT INTO routing_history (id, task_id, selected_backend, reason, fallback_applied, budget_downgrade_applied, decided_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    id,
                    task_id.0.to_string(),
                    selected_backend,
                    reason,
                    fallback_applied as i32,
                    budget_downgrade_applied as i32,
                    Utc::now().to_rfc3339(),
                ],
            )
            .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(())
    }

    pub fn get_routing_history_for_task(
        &self,
        task_id: &TaskId,
    ) -> Result<Option<RoutingHistoryRow>, StorageError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, task_id, selected_backend, reason, fallback_applied, budget_downgrade_applied, decided_at
                 FROM routing_history WHERE task_id = ?1 ORDER BY decided_at DESC LIMIT 1",
            )
            .map_err(|e| StorageError::Database(e.to_string()))?;

        let result = stmt
            .query_row(params![task_id.0.to_string()], |row| {
                Ok(RoutingHistoryRow {
                    id: row.get(0)?,
                    task_id: row.get(1)?,
                    selected_backend: row.get(2)?,
                    reason: row.get(3)?,
                    fallback_applied: row.get::<_, i32>(4)? != 0,
                    budget_downgrade_applied: row.get::<_, i32>(5)? != 0,
                    decided_at: row.get(6)?,
                })
            })
            .optional()
            .map_err(|e| StorageError::Database(e.to_string()))?;

        Ok(result)
    }

    pub fn list_actions_for_task(
        &self,
        task_id: &TaskId,
    ) -> Result<Vec<PendingAction>, StorageError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, action_type, project_id, task_id, description, payload, status, created_at
                 FROM pending_actions WHERE task_id = ?1 ORDER BY created_at DESC",
            )
            .map_err(|e| StorageError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![task_id.0.to_string()], |row| {
                Ok(PendingActionRow {
                    id: row.get(0)?,
                    action_type: row.get(1)?,
                    project_id: row.get(2)?,
                    task_id: row.get(3)?,
                    description: row.get(4)?,
                    payload: row.get(5)?,
                    status: row.get(6)?,
                    created_at: row.get(7)?,
                })
            })
            .map_err(|e| StorageError::Database(e.to_string()))?;

        let mut actions = Vec::new();
        for row in rows {
            let row = row.map_err(|e| StorageError::Database(e.to_string()))?;
            actions.push(row.into_pending_action()?);
        }
        Ok(actions)
    }

    /// Count tasks by status for a project.
    pub fn count_tasks_by_status(
        &self,
        project_id: &ProjectId,
    ) -> Result<Vec<(TaskStatus, usize)>, StorageError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT status, COUNT(*) FROM tasks WHERE project_id = ?1 GROUP BY status",
            )
            .map_err(|e| StorageError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![project_id.0.to_string()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(|e| StorageError::Database(e.to_string()))?;

        let mut counts = Vec::new();
        for row in rows {
            let (status_str, count) = row.map_err(|e| StorageError::Database(e.to_string()))?;
            let status: TaskStatus = serde_json::from_str(&status_str)
                .map_err(|e| StorageError::Serialization(e.to_string()))?;
            counts.push((status, count as usize));
        }
        Ok(counts)
    }

    // -----------------------------------------------------------------------
    // Task outputs
    // -----------------------------------------------------------------------

    /// Store execution output for a task.
    pub fn insert_task_output(
        &self,
        task_id: &TaskId,
        backend_id: &str,
        output: &str,
        structured_output: Option<&serde_json::Value>,
        model: Option<&str>,
        cost_cents: i64,
        input_tokens: u64,
        output_tokens: u64,
    ) -> Result<(), StorageError> {
        let id = uuid::Uuid::new_v4().to_string();
        let structured = structured_output.map(|v| v.to_string());
        self.conn
            .execute(
                "INSERT INTO task_outputs (id, task_id, backend_id, output, structured_output, model, cost_cents, input_tokens, output_tokens, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    id,
                    task_id.0.to_string(),
                    backend_id,
                    output,
                    structured,
                    model,
                    cost_cents,
                    input_tokens as i64,
                    output_tokens as i64,
                    Utc::now().to_rfc3339(),
                ],
            )
            .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(())
    }

    /// Retrieve the most recent output for a task.
    pub fn get_task_output(&self, task_id: &TaskId) -> Result<Option<TaskOutputRow>, StorageError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, task_id, backend_id, output, structured_output, model, cost_cents, input_tokens, output_tokens, created_at
                 FROM task_outputs WHERE task_id = ?1 ORDER BY created_at DESC LIMIT 1",
            )
            .map_err(|e| StorageError::Database(e.to_string()))?;

        let result = stmt
            .query_row(params![task_id.0.to_string()], |row| {
                Ok(TaskOutputRow {
                    id: row.get(0)?,
                    task_id: row.get(1)?,
                    backend_id: row.get(2)?,
                    output: row.get(3)?,
                    structured_output: row.get(4)?,
                    model: row.get(5)?,
                    cost_cents: row.get(6)?,
                    input_tokens: row.get(7)?,
                    output_tokens: row.get(8)?,
                    created_at: row.get(9)?,
                })
            })
            .optional()
            .map_err(|e| StorageError::Database(e.to_string()))?;

        Ok(result)
    }

    // -----------------------------------------------------------------------
    // Spending trends
    // -----------------------------------------------------------------------

    /// Aggregated spend per month for the last N months.
    /// Returns Vec<(year_month, total_cents)> ordered newest first.
    pub fn spend_by_month(&self, n_months: u32) -> Result<Vec<(String, MoneyAmount)>, StorageError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT SUBSTR(recorded_at, 1, 7) as ym, SUM(cost_cents) as total
                 FROM usage_records
                 GROUP BY ym
                 ORDER BY ym DESC
                 LIMIT ?1",
            )
            .map_err(|e| StorageError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![n_months as i64], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(|e| StorageError::Database(e.to_string()))?;

        let mut result = Vec::new();
        for row in rows {
            let (ym, cents) = row.map_err(|e| StorageError::Database(e.to_string()))?;
            result.push((ym, MoneyAmount::from_cents(cents)));
        }
        Ok(result)
    }

    /// Spend per backend per month for the last N months.
    pub fn spend_by_backend_month(
        &self,
        n_months: u32,
    ) -> Result<Vec<(String, String, MoneyAmount)>, StorageError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT SUBSTR(recorded_at, 1, 7) as ym, backend_id, SUM(cost_cents) as total
                 FROM usage_records
                 GROUP BY ym, backend_id
                 ORDER BY ym DESC, total DESC
                 LIMIT ?1",
            )
            .map_err(|e| StorageError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![(n_months * 10) as i64], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })
            .map_err(|e| StorageError::Database(e.to_string()))?;

        let mut result = Vec::new();
        for row in rows {
            let (ym, backend, cents) = row.map_err(|e| StorageError::Database(e.to_string()))?;
            result.push((ym, backend, MoneyAmount::from_cents(cents)));
        }
        Ok(result)
    }

    /// Spend per project per month for the last N months.
    pub fn spend_by_project_month(
        &self,
        n_months: u32,
    ) -> Result<Vec<(String, ProjectId, MoneyAmount)>, StorageError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT SUBSTR(recorded_at, 1, 7) as ym, project_id, SUM(cost_cents) as total
                 FROM usage_records
                 GROUP BY ym, project_id
                 ORDER BY ym DESC, total DESC
                 LIMIT ?1",
            )
            .map_err(|e| StorageError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![(n_months * 10) as i64], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })
            .map_err(|e| StorageError::Database(e.to_string()))?;

        let mut result = Vec::new();
        for row in rows {
            let (ym, pid_str, cents) = row.map_err(|e| StorageError::Database(e.to_string()))?;
            let pid = uuid::Uuid::parse_str(&pid_str)
                .map_err(|e| StorageError::Serialization(e.to_string()))?;
            result.push((ym, ProjectId(pid), MoneyAmount::from_cents(cents)));
        }
        Ok(result)
    }

    /// Count pending actions for a project.
    pub fn count_pending_actions_for_project(
        &self,
        project_id: &ProjectId,
    ) -> Result<usize, StorageError> {
        let count: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM pending_actions WHERE project_id = ?1 AND status = '\"Pending\"'",
                params![project_id.0.to_string()],
                |row| row.get(0),
            )
            .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(count as usize)
    }

    // -----------------------------------------------------------------------
    // Task dependencies
    // -----------------------------------------------------------------------

    /// Insert a dependency: `task_id` depends on `depends_on_task_id`.
    pub fn insert_task_dependency(
        &self,
        task_id: &TaskId,
        depends_on_task_id: &TaskId,
    ) -> Result<(), StorageError> {
        self.conn
            .execute(
                "INSERT OR IGNORE INTO task_dependencies (task_id, depends_on_task_id) VALUES (?1, ?2)",
                params![task_id.0.to_string(), depends_on_task_id.0.to_string()],
            )
            .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(())
    }

    /// Get all task IDs that the given task depends on.
    pub fn get_task_dependencies(&self, task_id: &TaskId) -> Result<Vec<TaskId>, StorageError> {
        let mut stmt = self
            .conn
            .prepare("SELECT depends_on_task_id FROM task_dependencies WHERE task_id = ?1")
            .map_err(|e| StorageError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![task_id.0.to_string()], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|e| StorageError::Database(e.to_string()))?;

        let mut deps = Vec::new();
        for row in rows {
            let id_str = row.map_err(|e| StorageError::Database(e.to_string()))?;
            let uuid = uuid::Uuid::parse_str(&id_str)
                .map_err(|e| StorageError::Serialization(e.to_string()))?;
            deps.push(TaskId(uuid));
        }
        Ok(deps)
    }

    /// Check if all dependencies for a task are in Completed status.
    pub fn all_dependencies_completed(&self, task_id: &TaskId) -> Result<bool, StorageError> {
        let count: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM task_dependencies td
                 JOIN tasks t ON t.id = td.depends_on_task_id
                 WHERE td.task_id = ?1 AND t.status != '\"Completed\"'",
                params![task_id.0.to_string()],
                |row| row.get(0),
            )
            .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(count == 0)
    }

    // -----------------------------------------------------------------------
    // Task queue operations
    // -----------------------------------------------------------------------

    /// Mark a task as queued with current timestamp.
    pub fn queue_task(&self, task_id: &TaskId) -> Result<(), StorageError> {
        let now = Utc::now().to_rfc3339();
        let status_str = serde_json::to_string(&TaskStatus::Queued)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let affected = self
            .conn
            .execute(
                "UPDATE tasks SET status = ?1, queued_at = ?2, updated_at = ?3 WHERE id = ?4",
                params![status_str, now, now, task_id.0.to_string()],
            )
            .map_err(|e| StorageError::Database(e.to_string()))?;
        if affected == 0 {
            return Err(StorageError::NotFound(format!("task {}", task_id.0)));
        }
        Ok(())
    }

    /// List all queued tasks, ordered by priority rank (ascending) then queued_at.
    pub fn list_queued_tasks(&self) -> Result<Vec<Task>, StorageError> {
        let status_str = serde_json::to_string(&TaskStatus::Queued)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, project_id, task_type, description, priority, status, backend_override, created_at, updated_at, queued_at, tags
                 FROM tasks WHERE status = ?1 ORDER BY priority ASC, queued_at ASC",
            )
            .map_err(|e| StorageError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![status_str], |row| {
                Ok(TaskRow {
                    id: row.get(0)?,
                    project_id: row.get(1)?,
                    task_type: row.get(2)?,
                    description: row.get(3)?,
                    priority: row.get(4)?,
                    status: row.get(5)?,
                    backend_override: row.get(6)?,
                    created_at: row.get(7)?,
                    updated_at: row.get(8)?,
                    queued_at: row.get(9)?,
                    tags: row.get(10)?,
                })
            })
            .map_err(|e| StorageError::Database(e.to_string()))?;

        let mut tasks: Vec<Task> = Vec::new();
        for row in rows {
            let row = row.map_err(|e| StorageError::Database(e.to_string()))?;
            tasks.push(row.into_task()?);
        }
        // Sort by priority rank then queued_at (SQLite sorts priority as JSON strings,
        // so we re-sort in Rust for correct numeric ordering)
        tasks.sort_by(|a, b| {
            a.priority
                .rank()
                .cmp(&b.priority.rank())
                .then_with(|| a.queued_at.cmp(&b.queued_at))
        });
        Ok(tasks)
    }

    /// Dequeue the highest-priority queued task (lowest rank, earliest queued_at).
    /// Returns the task and updates its status to Pending for processing.
    pub fn dequeue_next_task(&self) -> Result<Option<Task>, StorageError> {
        let queued = self.list_queued_tasks()?;
        if let Some(task) = queued.into_iter().next() {
            self.update_task_status(&task.id, TaskStatus::Pending)?;
            // Re-fetch to get updated status
            self.get_task(&task.id)
        } else {
            Ok(None)
        }
    }

    /// Count queued tasks.
    pub fn count_queued_tasks(&self) -> Result<usize, StorageError> {
        let status_str = serde_json::to_string(&TaskStatus::Queued)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let count: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM tasks WHERE status = ?1",
                params![status_str],
                |row| row.get(0),
            )
            .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(count as usize)
    }

    // -----------------------------------------------------------------------
    // Task tag search
    // -----------------------------------------------------------------------

    /// Search tasks by tag. Returns tasks that have the given tag.
    pub fn search_tasks_by_tag(&self, tag: &str) -> Result<Vec<Task>, StorageError> {
        // Tags are stored as JSON arrays, search using LIKE with the JSON-encoded tag value
        let pattern = format!("%\"{}\"%" , tag);
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, project_id, task_type, description, priority, status, backend_override, created_at, updated_at, queued_at, tags
                 FROM tasks WHERE tags LIKE ?1 ORDER BY created_at DESC",
            )
            .map_err(|e| StorageError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![pattern], |row| {
                Ok(TaskRow {
                    id: row.get(0)?,
                    project_id: row.get(1)?,
                    task_type: row.get(2)?,
                    description: row.get(3)?,
                    priority: row.get(4)?,
                    status: row.get(5)?,
                    backend_override: row.get(6)?,
                    created_at: row.get(7)?,
                    updated_at: row.get(8)?,
                    queued_at: row.get(9)?,
                    tags: row.get(10)?,
                })
            })
            .map_err(|e| StorageError::Database(e.to_string()))?;

        let mut tasks = Vec::new();
        for row in rows {
            let row = row.map_err(|e| StorageError::Database(e.to_string()))?;
            tasks.push(row.into_task()?);
        }
        Ok(tasks)
    }

    // -----------------------------------------------------------------------
    // Rate limiting
    // -----------------------------------------------------------------------

    /// Record a request for rate limiting purposes.
    pub fn record_rate_limit_request(&self, backend_id: &str) -> Result<(), StorageError> {
        let id = uuid::Uuid::new_v4().to_string();
        self.conn
            .execute(
                "INSERT INTO rate_limit_log (id, backend_id, recorded_at) VALUES (?1, ?2, ?3)",
                params![id, backend_id, Utc::now().to_rfc3339()],
            )
            .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(())
    }

    /// Count requests for a backend in the last N seconds.
    pub fn count_recent_requests(&self, backend_id: &str, window_secs: u64) -> Result<u32, StorageError> {
        let cutoff = (Utc::now() - chrono::Duration::seconds(window_secs as i64)).to_rfc3339();
        let count: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM rate_limit_log WHERE backend_id = ?1 AND recorded_at >= ?2",
                params![backend_id, cutoff],
                |row| row.get(0),
            )
            .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(count as u32)
    }

    /// Prune old rate limit log entries (older than window_secs).
    pub fn prune_rate_limit_log(&self, window_secs: u64) -> Result<usize, StorageError> {
        let cutoff = (Utc::now() - chrono::Duration::seconds(window_secs as i64)).to_rfc3339();
        let affected = self
            .conn
            .execute(
                "DELETE FROM rate_limit_log WHERE recorded_at < ?1",
                params![cutoff],
            )
            .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(affected)
    }

    // -----------------------------------------------------------------------
    // Circuit breaker state
    // -----------------------------------------------------------------------

    /// Get circuit breaker state for a backend.
    pub fn get_circuit_breaker_state(&self, backend_id: &str) -> Result<Option<CircuitBreakerState>, StorageError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT backend_id, consecutive_failures, last_failure_at, tripped_at, state
                 FROM circuit_breaker_state WHERE backend_id = ?1",
            )
            .map_err(|e| StorageError::Database(e.to_string()))?;

        let result = stmt
            .query_row(params![backend_id], |row| {
                Ok(CircuitBreakerState {
                    backend_id: row.get(0)?,
                    consecutive_failures: row.get::<_, i64>(1)? as u32,
                    last_failure_at: row.get(2)?,
                    tripped_at: row.get(3)?,
                    state: row.get(4)?,
                })
            })
            .optional()
            .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(result)
    }

    /// Record a backend failure, incrementing the consecutive failure counter.
    pub fn record_backend_failure(&self, backend_id: &str, failure_threshold: u32) -> Result<CircuitBreakerState, StorageError> {
        let now = Utc::now().to_rfc3339();
        let existing = self.get_circuit_breaker_state(backend_id)?;

        let (new_failures, new_state, tripped_at) = match existing {
            Some(ref s) if s.state == "Open" => {
                // Already tripped, just update last_failure_at
                (s.consecutive_failures + 1, "Open".to_string(), s.tripped_at.clone())
            }
            Some(ref s) => {
                let failures = s.consecutive_failures + 1;
                if failures >= failure_threshold {
                    (failures, "Open".to_string(), Some(now.clone()))
                } else {
                    (failures, "Closed".to_string(), None)
                }
            }
            None => {
                let failures = 1u32;
                if failures >= failure_threshold {
                    (failures, "Open".to_string(), Some(now.clone()))
                } else {
                    (failures, "Closed".to_string(), None)
                }
            }
        };

        self.conn
            .execute(
                "INSERT INTO circuit_breaker_state (backend_id, consecutive_failures, last_failure_at, tripped_at, state)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(backend_id) DO UPDATE SET
                    consecutive_failures = ?2, last_failure_at = ?3, tripped_at = ?4, state = ?5",
                params![backend_id, new_failures as i64, now, tripped_at, new_state],
            )
            .map_err(|e| StorageError::Database(e.to_string()))?;

        Ok(CircuitBreakerState {
            backend_id: backend_id.to_string(),
            consecutive_failures: new_failures,
            last_failure_at: Some(now),
            tripped_at,
            state: new_state,
        })
    }

    /// Record a backend success, resetting the circuit breaker.
    pub fn record_backend_success(&self, backend_id: &str) -> Result<(), StorageError> {
        self.conn
            .execute(
                "INSERT INTO circuit_breaker_state (backend_id, consecutive_failures, last_failure_at, tripped_at, state)
                 VALUES (?1, 0, NULL, NULL, 'Closed')
                 ON CONFLICT(backend_id) DO UPDATE SET
                    consecutive_failures = 0, last_failure_at = NULL, tripped_at = NULL, state = 'Closed'",
                params![backend_id],
            )
            .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(())
    }

    /// Check if a circuit breaker has recovered (cooldown elapsed).
    pub fn check_circuit_breaker_recovery(&self, backend_id: &str, cooldown_secs: u64) -> Result<bool, StorageError> {
        let state = self.get_circuit_breaker_state(backend_id)?;
        match state {
            Some(s) if s.state == "Open" => {
                if let Some(ref tripped) = s.tripped_at {
                    let tripped_time = chrono::DateTime::parse_from_rfc3339(tripped)
                        .map_err(|e| StorageError::Serialization(e.to_string()))?
                        .with_timezone(&Utc);
                    let elapsed = (Utc::now() - tripped_time).num_seconds() as u64;
                    Ok(elapsed >= cooldown_secs)
                } else {
                    Ok(false)
                }
            }
            _ => Ok(true), // Closed or not tracked = available
        }
    }

    // -----------------------------------------------------------------------
    // Concurrent task counting
    // -----------------------------------------------------------------------

    /// Count currently running tasks globally.
    pub fn count_running_tasks(&self) -> Result<u32, StorageError> {
        let status_str = serde_json::to_string(&TaskStatus::Running)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let count: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM tasks WHERE status = ?1",
                params![status_str],
                |row| row.get(0),
            )
            .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(count as u32)
    }

    /// Count currently running tasks for a specific project.
    pub fn count_running_tasks_for_project(&self, project_id: &ProjectId) -> Result<u32, StorageError> {
        let status_str = serde_json::to_string(&TaskStatus::Running)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let count: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM tasks WHERE status = ?1 AND project_id = ?2",
                params![status_str, project_id.0.to_string()],
                |row| row.get(0),
            )
            .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(count as u32)
    }

    /// Count pending tasks grouped by project.
    pub fn count_pending_tasks_by_project(&self) -> Result<std::collections::HashMap<ProjectId, usize>, StorageError> {
        let status_str = serde_json::to_string(&TaskStatus::Pending)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        
        let mut stmt = self
            .conn
            .prepare("SELECT project_id, COUNT(*) FROM tasks WHERE status = ?1 GROUP BY project_id")
            .map_err(|e| StorageError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![status_str], |row| {
                let project_id: String = row.get(0)?;
                let count: i64 = row.get(1)?;
                Ok((project_id, count as usize))
            })
            .map_err(|e| StorageError::Database(e.to_string()))?;

        let mut counts = std::collections::HashMap::new();
        for row in rows {
            let (project_id_str, count) = row.map_err(|e| StorageError::Database(e.to_string()))?;
            match uuid::Uuid::parse_str(&project_id_str) {
                Ok(uuid) => {
                    counts.insert(ProjectId(uuid), count);
                }
                Err(e) => {
                    tracing::warn!("Skipping task with invalid project_id '{}': {}", project_id_str, e);
                }
            }
        }
        Ok(counts)
    }

    /// Count currently running tasks for a specific backend (via routing_history).
    pub fn count_running_tasks_for_backend(&self, backend_id: &str) -> Result<u32, StorageError> {
        let status_str = serde_json::to_string(&TaskStatus::Running)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let count: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM tasks t
                 INNER JOIN routing_history r ON r.task_id = t.id
                 WHERE t.status = ?1 AND r.selected_backend = ?2",
                params![status_str, backend_id],
                |row| row.get(0),
            )
            .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(count as u32)
    }

    // -----------------------------------------------------------------------
    // Webhook deliveries
    // -----------------------------------------------------------------------

    /// Insert a webhook delivery record.
    pub fn insert_webhook_delivery(
        &self,
        delivery: &crate::models::event::WebhookDelivery,
    ) -> Result<(), StorageError> {
        let event_type_str = serde_json::to_string(&delivery.event_type)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let payload_str = serde_json::to_string(&delivery.payload)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        self.conn
            .execute(
                "INSERT INTO webhook_deliveries (id, webhook_name, url, event_type, payload, status_code, success, error, delivered_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    delivery.id,
                    delivery.webhook_name,
                    delivery.url,
                    event_type_str,
                    payload_str,
                    delivery.status_code.map(|c| c as i64),
                    delivery.success as i64,
                    delivery.error,
                    delivery.delivered_at.to_rfc3339(),
                ],
            )
            .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(())
    }

    /// List webhook deliveries, most recent first.
    pub fn list_webhook_deliveries(&self, limit: usize) -> Result<Vec<crate::models::event::WebhookDelivery>, StorageError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, webhook_name, url, event_type, payload, status_code, success, error, delivered_at
                 FROM webhook_deliveries ORDER BY delivered_at DESC LIMIT ?1",
            )
            .map_err(|e| StorageError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![limit as i64], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<i64>>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, String>(8)?,
                ))
            })
            .map_err(|e| StorageError::Database(e.to_string()))?;

        let mut deliveries = Vec::new();
        for row in rows {
            let (id, webhook_name, url, event_type_str, payload_str, status_code, success, error, delivered_at) =
                row.map_err(|e| StorageError::Database(e.to_string()))?;
            let event_type: EventType = serde_json::from_str(&event_type_str)
                .map_err(|e| StorageError::Serialization(e.to_string()))?;
            let payload: serde_json::Value = match payload_str {
                Some(ref s) => serde_json::from_str(s)
                    .map_err(|e| StorageError::Serialization(e.to_string()))?,
                None => serde_json::Value::Null,
            };
            let delivered_at = chrono::DateTime::parse_from_rfc3339(&delivered_at)
                .map_err(|e| StorageError::Serialization(e.to_string()))?
                .with_timezone(&Utc);
            deliveries.push(crate::models::event::WebhookDelivery {
                id,
                webhook_name,
                url,
                event_type,
                payload,
                status_code: status_code.map(|c| c as u16),
                success: success != 0,
                error,
                delivered_at,
            });
        }
        Ok(deliveries)
    }

    // -----------------------------------------------------------------------
    // Event filtering
    // -----------------------------------------------------------------------

    /// List events with optional filters.
    pub fn list_events_filtered(
        &self,
        event_type: Option<&str>,
        project_id: Option<&ProjectId>,
        task_id: Option<&TaskId>,
        since: Option<&str>,
        until: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Event>, StorageError> {
        let mut sql = String::from(
            "SELECT id, event_type, project_id, task_id, payload, timestamp
             FROM events WHERE 1=1"
        );
        let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(et) = event_type {
            // Event types are stored as JSON strings like "\"TaskSubmitted\""
            let et_json = serde_json::to_string(et)
                .map_err(|e| StorageError::Serialization(e.to_string()))?;
            params_vec.push(Box::new(et_json));
            sql.push_str(&format!(" AND event_type = ?{}", params_vec.len()));
        }
        if let Some(pid) = project_id {
            params_vec.push(Box::new(pid.0.to_string()));
            sql.push_str(&format!(" AND project_id = ?{}", params_vec.len()));
        }
        if let Some(tid) = task_id {
            params_vec.push(Box::new(tid.0.to_string()));
            sql.push_str(&format!(" AND task_id = ?{}", params_vec.len()));
        }
        if let Some(s) = since {
            params_vec.push(Box::new(s.to_string()));
            sql.push_str(&format!(" AND timestamp >= ?{}", params_vec.len()));
        }
        if let Some(u) = until {
            params_vec.push(Box::new(u.to_string()));
            sql.push_str(&format!(" AND timestamp <= ?{}", params_vec.len()));
        }

        params_vec.push(Box::new(limit as i64));
        sql.push_str(&format!(" ORDER BY timestamp DESC LIMIT ?{}", params_vec.len()));

        let mut stmt = self
            .conn
            .prepare(&sql)
            .map_err(|e| StorageError::Database(e.to_string()))?;

        let param_refs: Vec<&dyn rusqlite::types::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();

        let rows = stmt
            .query_map(param_refs.as_slice(), |row| {
                Ok(EventRow {
                    id: row.get(0)?,
                    event_type: row.get(1)?,
                    project_id: row.get(2)?,
                    task_id: row.get(3)?,
                    payload: row.get(4)?,
                    timestamp: row.get(5)?,
                })
            })
            .map_err(|e| StorageError::Database(e.to_string()))?;

        let mut events = Vec::new();
        for row in rows {
            let row = row.map_err(|e| StorageError::Database(e.to_string()))?;
            events.push(row.into_event()?);
        }
        Ok(events)
    }

    // -----------------------------------------------------------------------
    // Export / Import
    // -----------------------------------------------------------------------

    /// Export all data for a project as JSON-serializable structs.
    pub fn export_project_data(
        &self,
        project_id: &ProjectId,
    ) -> Result<ProjectExportData, StorageError> {
        let project = self
            .get_project(project_id)?
            .ok_or_else(|| StorageError::NotFound(format!("project {}", project_id)))?;

        let tasks = self.list_tasks_by_project(project_id)?;
        let usage_records = self.list_usage_records(Some(project_id), None, None, 10000)?;
        let actions = self.list_actions_for_project(project_id)?;

        Ok(ProjectExportData {
            project,
            tasks,
            usage_records,
            actions,
        })
    }

    /// List all actions for a project (all statuses).
    pub fn list_actions_for_project(
        &self,
        project_id: &ProjectId,
    ) -> Result<Vec<PendingAction>, StorageError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, action_type, project_id, task_id, description, payload, status, created_at
                 FROM pending_actions WHERE project_id = ?1 ORDER BY created_at DESC",
            )
            .map_err(|e| StorageError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![project_id.0.to_string()], |row| {
                Ok(PendingActionRow {
                    id: row.get(0)?,
                    action_type: row.get(1)?,
                    project_id: row.get(2)?,
                    task_id: row.get(3)?,
                    description: row.get(4)?,
                    payload: row.get(5)?,
                    status: row.get(6)?,
                    created_at: row.get(7)?,
                })
            })
            .map_err(|e| StorageError::Database(e.to_string()))?;

        let mut actions = Vec::new();
        for row in rows {
            let row = row.map_err(|e| StorageError::Database(e.to_string()))?;
            actions.push(row.into_pending_action()?);
        }
        Ok(actions)
    }

    /// Import project data, skipping duplicates (by primary key).
    pub fn import_project_data(&self, data: &ProjectExportData) -> Result<ImportResult, StorageError> {
        let mut result = ImportResult::default();

        // Import project (skip if exists)
        match self.get_project(&data.project.id)? {
            Some(_) => result.skipped_project = true,
            None => {
                self.insert_project(&data.project)?;
                result.imported_project = true;
            }
        }

        // Import tasks
        for task in &data.tasks {
            match self.get_task(&task.id)? {
                Some(_) => result.skipped_tasks += 1,
                None => {
                    self.insert_task(task)?;
                    result.imported_tasks += 1;
                }
            }
        }

        // Import usage records
        for record in &data.usage_records {
            // Check if exists by trying to insert (use OR IGNORE)
            let result_inner = self.conn.execute(
                "INSERT OR IGNORE INTO usage_records (id, task_id, project_id, backend_id, input_tokens, output_tokens, cost_cents, model, recorded_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    record.id.0.to_string(),
                    record.task_id.0.to_string(),
                    record.project_id.0.to_string(),
                    record.backend_id.as_str(),
                    record.input_tokens as i64,
                    record.output_tokens as i64,
                    record.cost.cents,
                    record.model,
                    record.recorded_at.to_rfc3339(),
                ],
            ).map_err(|e| StorageError::Database(e.to_string()))?;

            if result_inner > 0 {
                result.imported_usage += 1;
            } else {
                result.skipped_usage += 1;
            }
        }

        // Import actions
        for action in &data.actions {
            let action_type_str = serde_json::to_string(&action.action_type)
                .map_err(|e| StorageError::Serialization(e.to_string()))?;
            let status_str = serde_json::to_string(&action.status)
                .map_err(|e| StorageError::Serialization(e.to_string()))?;
            let payload_str = serde_json::to_string(&action.payload)
                .map_err(|e| StorageError::Serialization(e.to_string()))?;

            let rows = self.conn.execute(
                "INSERT OR IGNORE INTO pending_actions (id, action_type, project_id, task_id, description, payload, status, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    action.id.0.to_string(),
                    action_type_str,
                    action.project_id.0.to_string(),
                    action.task_id.as_ref().map(|t| t.0.to_string()),
                    action.description,
                    payload_str,
                    status_str,
                    action.created_at.to_rfc3339(),
                ],
            ).map_err(|e| StorageError::Database(e.to_string()))?;

            if rows > 0 {
                result.imported_actions += 1;
            } else {
                result.skipped_actions += 1;
            }
        }

        Ok(result)
    }
}

/// Data exported from a project for backup/migration.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProjectExportData {
    pub project: Project,
    pub tasks: Vec<Task>,
    pub usage_records: Vec<UsageRecord>,
    pub actions: Vec<PendingAction>,
}

/// Result of an import operation.
#[derive(Debug, Default)]
pub struct ImportResult {
    pub imported_project: bool,
    pub skipped_project: bool,
    pub imported_tasks: usize,
    pub skipped_tasks: usize,
    pub imported_usage: usize,
    pub skipped_usage: usize,
    pub imported_actions: usize,
    pub skipped_actions: usize,
}

/// Thread-safe storage wrapper using a Mutex around the Connection.
/// Implements the budget governor's `UsageStore` trait for production use.
pub struct ThreadSafeStorage {
    conn: std::sync::Mutex<Connection>,
}

impl ThreadSafeStorage {
    pub fn open(path: &Path) -> Result<Self, StorageError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| StorageError::Database(format!("cannot create directory: {}", e)))?;
        }
        let conn =
            Connection::open(path).map_err(|e| StorageError::Database(e.to_string()))?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
            .map_err(|e| StorageError::Database(e.to_string()))?;

        let storage = Self {
            conn: std::sync::Mutex::new(conn),
        };
        storage.migrate()?;
        Ok(storage)
    }

    pub fn in_memory() -> Result<Self, StorageError> {
        let conn =
            Connection::open_in_memory().map_err(|e| StorageError::Database(e.to_string()))?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")
            .map_err(|e| StorageError::Database(e.to_string()))?;

        let storage = Self {
            conn: std::sync::Mutex::new(conn),
        };
        storage.migrate()?;
        Ok(storage)
    }

    fn migrate(&self) -> Result<(), StorageError> {
        let conn = self.conn.lock().map_err(|e| StorageError::Database(e.to_string()))?;

        let has_migrations = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='_migrations'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|e| StorageError::Database(e.to_string()))?;

        let current_version = if has_migrations > 0 {
            conn.query_row(
                "SELECT COALESCE(MAX(version), 0) FROM _migrations",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(0)
        } else {
            0
        };

        if current_version < 1 {
            conn.execute_batch(SCHEMA_V1)
                .map_err(|e| StorageError::Database(e.to_string()))?;
            conn.execute(
                "INSERT INTO _migrations (version, applied_at) VALUES (?1, ?2)",
                params![1, Utc::now().to_rfc3339()],
            )
            .map_err(|e| StorageError::Database(e.to_string()))?;
        }

        Ok(())
    }

    fn current_year_month() -> String {
        Utc::now().format("%Y-%m").to_string()
    }

    pub fn total_spend_month(&self, year_month: &str) -> Result<MoneyAmount, StorageError> {
        let conn = self.conn.lock().map_err(|e| StorageError::Database(e.to_string()))?;
        let cents: i64 = conn
            .query_row(
                "SELECT COALESCE(SUM(cost_cents), 0) FROM usage_records WHERE recorded_at LIKE ?1",
                params![format!("{}%", year_month)],
                |row| row.get(0),
            )
            .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(MoneyAmount::from_cents(cents))
    }

    pub fn backend_spend_month(
        &self,
        backend_id: &BackendId,
        year_month: &str,
    ) -> Result<MoneyAmount, StorageError> {
        let conn = self.conn.lock().map_err(|e| StorageError::Database(e.to_string()))?;
        let cents: i64 = conn
            .query_row(
                "SELECT COALESCE(SUM(cost_cents), 0) FROM usage_records WHERE backend_id = ?1 AND recorded_at LIKE ?2",
                params![backend_id.as_str(), format!("{}%", year_month)],
                |row| row.get(0),
            )
            .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(MoneyAmount::from_cents(cents))
    }

    pub fn project_spend_month(
        &self,
        project_id: &ProjectId,
        year_month: &str,
    ) -> Result<MoneyAmount, StorageError> {
        let conn = self.conn.lock().map_err(|e| StorageError::Database(e.to_string()))?;
        let cents: i64 = conn
            .query_row(
                "SELECT COALESCE(SUM(cost_cents), 0) FROM usage_records WHERE project_id = ?1 AND recorded_at LIKE ?2",
                params![project_id.0.to_string(), format!("{}%", year_month)],
                |row| row.get(0),
            )
            .map_err(|e| StorageError::Database(e.to_string()))?;
        Ok(MoneyAmount::from_cents(cents))
    }
}

#[async_trait]
impl UsageStore for ThreadSafeStorage {
    async fn total_spend_current_month(&self) -> Result<MoneyAmount, BudgetError> {
        let ym = Self::current_year_month();
        self.total_spend_month(&ym)
            .map_err(|e| BudgetError::Storage(e.to_string()))
    }

    async fn backend_spend_current_month(
        &self,
        backend: &BackendId,
    ) -> Result<MoneyAmount, BudgetError> {
        let ym = Self::current_year_month();
        self.backend_spend_month(backend, &ym)
            .map_err(|e| BudgetError::Storage(e.to_string()))
    }

    async fn project_spend_current_month(
        &self,
        project: &ProjectId,
    ) -> Result<MoneyAmount, BudgetError> {
        let ym = Self::current_year_month();
        self.project_spend_month(project, &ym)
            .map_err(|e| BudgetError::Storage(e.to_string()))
    }
}

/// Circuit breaker state for a backend.
#[derive(Debug, Clone)]
pub struct CircuitBreakerState {
    pub backend_id: String,
    pub consecutive_failures: u32,
    pub last_failure_at: Option<String>,
    pub tripped_at: Option<String>,
    pub state: String,
}

// ---------------------------------------------------------------------------
// Row mapping helpers
// ---------------------------------------------------------------------------

pub struct RoutingHistoryRow {
    pub id: String,
    pub task_id: String,
    pub selected_backend: String,
    pub reason: String,
    pub fallback_applied: bool,
    pub budget_downgrade_applied: bool,
    pub decided_at: String,
}

/// Task execution output row from the database.
pub struct TaskOutputRow {
    pub id: String,
    pub task_id: String,
    pub backend_id: String,
    pub output: String,
    pub structured_output: Option<String>,
    pub model: Option<String>,
    pub cost_cents: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub created_at: String,
}

struct UsageRecordRow {
    id: String,
    task_id: String,
    project_id: String,
    backend_id: String,
    input_tokens: i64,
    output_tokens: i64,
    cost_cents: i64,
    model: Option<String>,
    recorded_at: String,
}

impl UsageRecordRow {
    fn into_usage_record(self) -> Result<UsageRecord, StorageError> {
        let id = uuid::Uuid::parse_str(&self.id)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let task_id = uuid::Uuid::parse_str(&self.task_id)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let project_id = uuid::Uuid::parse_str(&self.project_id)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let recorded_at = chrono::DateTime::parse_from_rfc3339(&self.recorded_at)
            .map_err(|e| StorageError::Serialization(e.to_string()))?
            .with_timezone(&Utc);

        Ok(UsageRecord {
            id: crate::models::UsageId(id),
            task_id: TaskId(task_id),
            project_id: ProjectId(project_id),
            backend_id: BackendId::new(&self.backend_id),
            input_tokens: self.input_tokens as u64,
            output_tokens: self.output_tokens as u64,
            cost: MoneyAmount::from_cents(self.cost_cents),
            model: self.model,
            recorded_at,
        })
    }
}

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

struct TaskRow {
    id: String,
    project_id: String,
    task_type: String,
    description: String,
    priority: String,
    status: String,
    backend_override: Option<String>,
    created_at: String,
    updated_at: String,
    queued_at: Option<String>,
    tags: Option<String>,
}

impl TaskRow {
    fn into_task(self) -> Result<Task, StorageError> {
        let id = uuid::Uuid::parse_str(&self.id)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let project_id = uuid::Uuid::parse_str(&self.project_id)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let task_type: TaskType = serde_json::from_str(&self.task_type)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let priority: Priority = serde_json::from_str(&self.priority)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let status: TaskStatus = serde_json::from_str(&self.status)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let backend_override = self.backend_override.map(BackendId::new);
        let created_at = chrono::DateTime::parse_from_rfc3339(&self.created_at)
            .map_err(|e| StorageError::Serialization(e.to_string()))?
            .with_timezone(&Utc);
        let updated_at = chrono::DateTime::parse_from_rfc3339(&self.updated_at)
            .map_err(|e| StorageError::Serialization(e.to_string()))?
            .with_timezone(&Utc);
        let queued_at = self
            .queued_at
            .map(|s| {
                chrono::DateTime::parse_from_rfc3339(&s)
                    .map(|dt| dt.with_timezone(&Utc))
                    .map_err(|e| StorageError::Serialization(e.to_string()))
            })
            .transpose()?;

        let tags: Vec<String> = match self.tags {
            Some(ref s) => serde_json::from_str(s)
                .map_err(|e| StorageError::Serialization(e.to_string()))?,
            None => Vec::new(),
        };

        Ok(Task {
            id: TaskId(id),
            project_id: ProjectId(project_id),
            task_type,
            description: self.description,
            priority,
            status,
            backend_override,
            created_at,
            updated_at,
            queued_at,
            tags,
        })
    }
}

struct EventRow {
    id: String,
    event_type: String,
    project_id: Option<String>,
    task_id: Option<String>,
    payload: Option<String>,
    timestamp: String,
}

impl EventRow {
    fn into_event(self) -> Result<Event, StorageError> {
        let id = uuid::Uuid::parse_str(&self.id)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let event_type: EventType = serde_json::from_str(&self.event_type)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let project_id = self
            .project_id
            .map(|s| uuid::Uuid::parse_str(&s).map(ProjectId))
            .transpose()
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let task_id = self
            .task_id
            .map(|s| uuid::Uuid::parse_str(&s).map(TaskId))
            .transpose()
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let payload: serde_json::Value = match self.payload {
            Some(ref s) => serde_json::from_str(s)
                .map_err(|e| StorageError::Serialization(e.to_string()))?,
            None => serde_json::Value::Null,
        };
        let timestamp = chrono::DateTime::parse_from_rfc3339(&self.timestamp)
            .map_err(|e| StorageError::Serialization(e.to_string()))?
            .with_timezone(&Utc);

        Ok(Event {
            id: EventId(id),
            event_type,
            project_id,
            task_id,
            payload,
            timestamp,
        })
    }
}

struct PendingActionRow {
    id: String,
    action_type: String,
    project_id: String,
    task_id: Option<String>,
    description: String,
    payload: Option<String>,
    status: String,
    created_at: String,
}

impl PendingActionRow {
    fn into_pending_action(self) -> Result<PendingAction, StorageError> {
        let id = uuid::Uuid::parse_str(&self.id)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let action_type: PendingActionType = serde_json::from_str(&self.action_type)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let project_id = uuid::Uuid::parse_str(&self.project_id)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let task_id = self
            .task_id
            .map(|s| uuid::Uuid::parse_str(&s).map(TaskId))
            .transpose()
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let status: ActionStatus = serde_json::from_str(&self.status)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let payload: serde_json::Value = match self.payload {
            Some(ref s) => serde_json::from_str(s)
                .map_err(|e| StorageError::Serialization(e.to_string()))?,
            None => serde_json::Value::Null,
        };
        let created_at = chrono::DateTime::parse_from_rfc3339(&self.created_at)
            .map_err(|e| StorageError::Serialization(e.to_string()))?
            .with_timezone(&Utc);

        Ok(PendingAction {
            id: ActionId(id),
            action_type,
            project_id: ProjectId(project_id),
            task_id,
            description: self.description,
            payload,
            status,
            created_at,
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
    use crate::models::event::EventType;
    use crate::models::policy::{ActionStatus, PendingAction, PendingActionType};
    use crate::models::project::Project;
    use crate::models::task::Task;

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
    fn get_project_by_name() {
        let storage = SqliteStorage::in_memory().unwrap();
        let project = Project::new("my-project", "/tmp/my");
        storage.insert_project(&project).unwrap();

        let fetched = storage.get_project_by_name("my-project").unwrap().unwrap();
        assert_eq!(fetched.id, project.id);

        assert!(storage.get_project_by_name("nonexistent").unwrap().is_none());
    }

    #[test]
    fn delete_nonexistent_project_returns_not_found() {
        let storage = SqliteStorage::in_memory().unwrap();
        let result = storage.delete_project(&ProjectId::new());
        assert!(matches!(result, Err(StorageError::NotFound(_))));
    }

    #[test]
    fn task_crud_roundtrip() {
        let storage = SqliteStorage::in_memory().unwrap();

        let project = Project::new("p", "/tmp/p");
        storage.insert_project(&project).unwrap();

        let task = Task::new(project.id.clone(), TaskType::Planning, "plan the module");
        storage.insert_task(&task).unwrap();

        let fetched = storage.get_task(&task.id).unwrap().unwrap();
        assert_eq!(fetched.description, "plan the module");
        assert_eq!(fetched.task_type, TaskType::Planning);
        assert_eq!(fetched.status, TaskStatus::Pending);

        storage.update_task_status(&task.id, TaskStatus::Running).unwrap();
        let fetched = storage.get_task(&task.id).unwrap().unwrap();
        assert_eq!(fetched.status, TaskStatus::Running);

        let tasks = storage.list_tasks_by_project(&project.id).unwrap();
        assert_eq!(tasks.len(), 1);
    }

    #[test]
    fn usage_spend_queries() {
        let storage = SqliteStorage::in_memory().unwrap();

        let project = Project::new("proj", "/tmp/p");
        storage.insert_project(&project).unwrap();

        let task = Task::new(project.id.clone(), TaskType::Planning, "test");
        storage.insert_task(&task).unwrap();

        let record = UsageRecord::new(
            task.id.clone(),
            project.id.clone(),
            BackendId::new("claude"),
            100,
            50,
            MoneyAmount::from_cents(500),
        );
        storage.insert_usage(&record).unwrap();

        let total = storage.total_spend_month(&Utc::now().format("%Y-%m").to_string()).unwrap();
        assert_eq!(total.cents, 500);
    }

    #[test]
    fn event_insert_and_query() {
        let storage = SqliteStorage::in_memory().unwrap();

        let project = Project::new("p", "/tmp/p");
        storage.insert_project(&project).unwrap();

        let event = Event::new(EventType::TaskSubmitted, serde_json::json!({"info": "test"}))
            .with_project(project.id.clone());
        storage.insert_event(&event).unwrap();

        let events = storage.list_events_recent(10).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, EventType::TaskSubmitted);

        let proj_events = storage.list_events_by_project(&project.id, 10).unwrap();
        assert_eq!(proj_events.len(), 1);
    }

    #[test]
    fn pending_action_lifecycle() {
        let storage = SqliteStorage::in_memory().unwrap();

        let project = Project::new("p", "/tmp/p");
        storage.insert_project(&project).unwrap();

        let action = PendingAction::new(
            PendingActionType::ReviewRequest,
            project.id.clone(),
            "review changes",
        );
        storage.insert_pending_action(&action).unwrap();

        let pending = storage.list_pending_actions().unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].description, "review changes");

        storage
            .update_action_status(&action.id, ActionStatus::Approved)
            .unwrap();

        // After approval, it should no longer appear in pending list
        let pending = storage.list_pending_actions().unwrap();
        assert!(pending.is_empty());
    }

    #[test]
    fn routing_history_insert() {
        let storage = SqliteStorage::in_memory().unwrap();

        let project = Project::new("p", "/tmp/p");
        storage.insert_project(&project).unwrap();

        let task = Task::new(project.id.clone(), TaskType::Planning, "test");
        storage.insert_task(&task).unwrap();

        storage
            .insert_routing_history(&task.id, "claude", "TaskTypeDefault", false, false)
            .unwrap();
    }

    #[test]
    fn thread_safe_storage_works() {
        let storage = ThreadSafeStorage::in_memory().unwrap();
        let ym = ThreadSafeStorage::current_year_month();
        let total = storage.total_spend_month(&ym).unwrap();
        assert_eq!(total, MoneyAmount::ZERO);
    }

    #[test]
    fn get_pending_action_by_id() {
        let storage = SqliteStorage::in_memory().unwrap();
        let project = Project::new("p", "/tmp/p");
        storage.insert_project(&project).unwrap();

        let action = PendingAction::new(
            PendingActionType::ReviewRequest,
            project.id.clone(),
            "review code",
        );
        storage.insert_pending_action(&action).unwrap();

        let fetched = storage.get_pending_action(&action.id).unwrap().unwrap();
        assert_eq!(fetched.description, "review code");
        assert_eq!(fetched.action_type, PendingActionType::ReviewRequest);

        // Non-existent ID returns None
        assert!(storage.get_pending_action(&ActionId::new()).unwrap().is_none());
    }

    #[test]
    fn list_all_actions_includes_all_statuses() {
        let storage = SqliteStorage::in_memory().unwrap();
        let project = Project::new("p", "/tmp/p");
        storage.insert_project(&project).unwrap();

        let a1 = PendingAction::new(PendingActionType::ReviewRequest, project.id.clone(), "a1");
        let a2 = PendingAction::new(PendingActionType::CommitSuggestion, project.id.clone(), "a2");
        storage.insert_pending_action(&a1).unwrap();
        storage.insert_pending_action(&a2).unwrap();

        storage.update_action_status(&a1.id, ActionStatus::Approved).unwrap();

        // list_pending_actions only returns pending
        let pending = storage.list_pending_actions().unwrap();
        assert_eq!(pending.len(), 1);

        // list_all_actions returns all
        let all = storage.list_all_actions(10).unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn routing_history_for_task() {
        let storage = SqliteStorage::in_memory().unwrap();
        let project = Project::new("p", "/tmp/p");
        storage.insert_project(&project).unwrap();

        let task = Task::new(project.id.clone(), TaskType::Planning, "test");
        storage.insert_task(&task).unwrap();

        // No history yet
        assert!(storage.get_routing_history_for_task(&task.id).unwrap().is_none());

        storage
            .insert_routing_history(&task.id, "claude", "TaskTypeDefault", false, true)
            .unwrap();

        let history = storage.get_routing_history_for_task(&task.id).unwrap().unwrap();
        assert_eq!(history.selected_backend, "claude");
        assert_eq!(history.reason, "TaskTypeDefault");
        assert!(!history.fallback_applied);
        assert!(history.budget_downgrade_applied);
    }

    #[test]
    fn actions_linked_to_task() {
        let storage = SqliteStorage::in_memory().unwrap();
        let project = Project::new("p", "/tmp/p");
        storage.insert_project(&project).unwrap();

        let task = Task::new(project.id.clone(), TaskType::Review, "review");
        storage.insert_task(&task).unwrap();

        let action = PendingAction::new(
            PendingActionType::ReviewRequest,
            project.id.clone(),
            "review findings",
        )
        .with_task(task.id.clone());
        storage.insert_pending_action(&action).unwrap();

        // Unlinked action
        let other = PendingAction::new(
            PendingActionType::BudgetApproval,
            project.id.clone(),
            "budget approval",
        );
        storage.insert_pending_action(&other).unwrap();

        let task_actions = storage.list_actions_for_task(&task.id).unwrap();
        assert_eq!(task_actions.len(), 1);
        assert_eq!(task_actions[0].description, "review findings");
    }

    #[test]
    fn list_usage_records_no_filter() {
        let storage = SqliteStorage::in_memory().unwrap();
        let project = Project::new("p", "/tmp/p");
        storage.insert_project(&project).unwrap();

        let task = Task::new(project.id.clone(), TaskType::Planning, "test");
        storage.insert_task(&task).unwrap();

        let record = UsageRecord::new(
            task.id.clone(),
            project.id.clone(),
            BackendId::new("claude"),
            100,
            50,
            MoneyAmount::from_cents(200),
        );
        storage.insert_usage(&record).unwrap();

        let records = storage.list_usage_records(None, None, None, 50).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].cost.cents, 200);
        assert_eq!(records[0].backend_id, BackendId::new("claude"));
    }

    #[test]
    fn list_usage_records_with_project_filter() {
        let storage = SqliteStorage::in_memory().unwrap();
        let p1 = Project::new("p1", "/tmp/p1");
        let p2 = Project::new("p2", "/tmp/p2");
        storage.insert_project(&p1).unwrap();
        storage.insert_project(&p2).unwrap();

        let t1 = Task::new(p1.id.clone(), TaskType::Planning, "t1");
        let t2 = Task::new(p2.id.clone(), TaskType::Review, "t2");
        storage.insert_task(&t1).unwrap();
        storage.insert_task(&t2).unwrap();

        let r1 = UsageRecord::new(t1.id.clone(), p1.id.clone(), BackendId::new("claude"), 100, 50, MoneyAmount::from_cents(100));
        let r2 = UsageRecord::new(t2.id.clone(), p2.id.clone(), BackendId::new("ollama"), 200, 100, MoneyAmount::ZERO);
        storage.insert_usage(&r1).unwrap();
        storage.insert_usage(&r2).unwrap();

        let filtered = storage.list_usage_records(Some(&p1.id), None, None, 50).unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].project_id, p1.id);
    }

    #[test]
    fn list_usage_records_with_backend_filter() {
        let storage = SqliteStorage::in_memory().unwrap();
        let project = Project::new("p", "/tmp/p");
        storage.insert_project(&project).unwrap();

        let t1 = Task::new(project.id.clone(), TaskType::Planning, "t1");
        let t2 = Task::new(project.id.clone(), TaskType::Summarization, "t2");
        storage.insert_task(&t1).unwrap();
        storage.insert_task(&t2).unwrap();

        let r1 = UsageRecord::new(t1.id.clone(), project.id.clone(), BackendId::new("claude"), 100, 50, MoneyAmount::from_cents(100));
        let r2 = UsageRecord::new(t2.id.clone(), project.id.clone(), BackendId::new("ollama"), 200, 100, MoneyAmount::ZERO);
        storage.insert_usage(&r1).unwrap();
        storage.insert_usage(&r2).unwrap();

        let claude_only = storage.list_usage_records(None, Some(&BackendId::new("claude")), None, 50).unwrap();
        assert_eq!(claude_only.len(), 1);
        assert_eq!(claude_only[0].backend_id, BackendId::new("claude"));
    }

    #[test]
    fn schema_v2_migration_creates_indexes() {
        let storage = SqliteStorage::in_memory().unwrap();

        // Verify migration version is 8 (V1-V8)
        let version: i64 = storage.conn.query_row(
            "SELECT MAX(version) FROM _migrations",
            [],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(version, 8);

        // Verify indexes exist
        let index_count: i64 = storage.conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name LIKE 'idx_%'",
            [],
            |row| row.get(0),
        ).unwrap();
        assert!(index_count >= 7, "expected at least 7 indexes, got {}", index_count);
    }

    #[test]
    fn count_tasks_and_pending_actions() {
        let storage = SqliteStorage::in_memory().unwrap();
        let project = Project::new("p", "/tmp/p");
        storage.insert_project(&project).unwrap();

        let t1 = Task::new(project.id.clone(), TaskType::Planning, "plan");
        let t2 = Task::new(project.id.clone(), TaskType::Review, "review");
        storage.insert_task(&t1).unwrap();
        storage.insert_task(&t2).unwrap();

        // Both tasks are Pending by default
        let counts = storage.count_tasks_by_status(&project.id).unwrap();
        let pending_count = counts.iter().find(|(s, _)| *s == TaskStatus::Pending).map(|(_, c)| *c).unwrap_or(0);
        assert_eq!(pending_count, 2);

        // Update one
        storage.update_task_status(&t1.id, TaskStatus::Completed).unwrap();
        let counts = storage.count_tasks_by_status(&project.id).unwrap();
        let completed = counts.iter().find(|(s, _)| *s == TaskStatus::Completed).map(|(_, c)| *c).unwrap_or(0);
        assert_eq!(completed, 1);

        // Pending actions count
        let action = PendingAction::new(PendingActionType::ReviewRequest, project.id.clone(), "r");
        storage.insert_pending_action(&action).unwrap();

        let count = storage.count_pending_actions_for_project(&project.id).unwrap();
        assert_eq!(count, 1);

        storage.update_action_status(&action.id, ActionStatus::Approved).unwrap();
        let count = storage.count_pending_actions_for_project(&project.id).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn count_pending_tasks_by_project_groups_correctly() {
        let storage = SqliteStorage::in_memory().unwrap();
        let p1 = Project::new("p1", "/tmp/p1");
        let p2 = Project::new("p2", "/tmp/p2");
        storage.insert_project(&p1).unwrap();
        storage.insert_project(&p2).unwrap();

        // Create tasks with different statuses and projects
        let t1 = Task::new(p1.id.clone(), TaskType::Planning, "p1 pending 1");
        let t2 = Task::new(p1.id.clone(), TaskType::Planning, "p1 pending 2");
        let t3 = Task::new(p1.id.clone(), TaskType::Planning, "p1 completed");
        let t4 = Task::new(p2.id.clone(), TaskType::Planning, "p2 pending");
        let t5 = Task::new(p2.id.clone(), TaskType::Planning, "p2 running");

        storage.insert_task(&t1).unwrap();
        storage.insert_task(&t2).unwrap();
        storage.insert_task(&t3).unwrap();
        storage.insert_task(&t4).unwrap();
        storage.insert_task(&t5).unwrap();

        // Update statuses
        storage.update_task_status(&t3.id, TaskStatus::Completed).unwrap();
        storage.update_task_status(&t5.id, TaskStatus::Running).unwrap();

        // Get counts
        let counts = storage.count_pending_tasks_by_project().unwrap();

        // p1 should have 2 pending (t1, t2), p2 should have 1 pending (t4)
        assert_eq!(counts.get(&p1.id), Some(&2));
        assert_eq!(counts.get(&p2.id), Some(&1));

        // Total pending tasks
        let total: usize = counts.values().sum();
        assert_eq!(total, 3);
    }

    #[test]
    fn count_pending_tasks_by_project_empty() {
        let storage = SqliteStorage::in_memory().unwrap();
        let counts = storage.count_pending_tasks_by_project().unwrap();
        assert!(counts.is_empty());
    }
}
