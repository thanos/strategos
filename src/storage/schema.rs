/// SQL statements for creating the Strategos database schema.
pub const SCHEMA_V1: &str = r#"
CREATE TABLE IF NOT EXISTS _migrations (
    version     INTEGER PRIMARY KEY,
    applied_at  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS projects (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    path        TEXT NOT NULL,
    privacy     TEXT NOT NULL DEFAULT 'Public',
    tags        TEXT,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS tasks (
    id              TEXT PRIMARY KEY,
    project_id      TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    task_type       TEXT NOT NULL,
    description     TEXT NOT NULL,
    priority        TEXT NOT NULL DEFAULT 'Normal',
    status          TEXT NOT NULL DEFAULT 'Pending',
    backend_override TEXT,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS usage_records (
    id            TEXT PRIMARY KEY,
    task_id       TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    project_id    TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    backend_id    TEXT NOT NULL,
    input_tokens  INTEGER NOT NULL,
    output_tokens INTEGER NOT NULL,
    cost_cents    INTEGER NOT NULL,
    model         TEXT,
    recorded_at   TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS events (
    id          TEXT PRIMARY KEY,
    event_type  TEXT NOT NULL,
    project_id  TEXT,
    task_id     TEXT,
    payload     TEXT,
    timestamp   TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS pending_actions (
    id          TEXT PRIMARY KEY,
    action_type TEXT NOT NULL,
    project_id  TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    task_id     TEXT,
    description TEXT NOT NULL,
    payload     TEXT,
    status      TEXT NOT NULL DEFAULT 'Pending',
    created_at  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS routing_history (
    id                        TEXT PRIMARY KEY,
    task_id                   TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    selected_backend          TEXT NOT NULL,
    reason                    TEXT NOT NULL,
    fallback_applied          INTEGER NOT NULL DEFAULT 0,
    budget_downgrade_applied  INTEGER NOT NULL DEFAULT 0,
    evaluated_backends        TEXT,
    decided_at                TEXT NOT NULL
);
"#;

/// V2: Add performance indexes for common query patterns.
pub const SCHEMA_V2: &str = r#"
CREATE INDEX IF NOT EXISTS idx_usage_records_recorded_at ON usage_records(recorded_at);
CREATE INDEX IF NOT EXISTS idx_usage_records_project_id ON usage_records(project_id);
CREATE INDEX IF NOT EXISTS idx_usage_records_backend_id ON usage_records(backend_id);
CREATE INDEX IF NOT EXISTS idx_tasks_project_status ON tasks(project_id, status);
CREATE INDEX IF NOT EXISTS idx_events_timestamp ON events(timestamp);
CREATE INDEX IF NOT EXISTS idx_pending_actions_status ON pending_actions(status);
CREATE INDEX IF NOT EXISTS idx_routing_history_task_id ON routing_history(task_id);
"#;

/// V3: Add task_outputs table for persistent storage of execution results.
pub const SCHEMA_V3: &str = r#"
CREATE TABLE IF NOT EXISTS task_outputs (
    id              TEXT PRIMARY KEY,
    task_id         TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    backend_id      TEXT NOT NULL,
    output          TEXT NOT NULL,
    structured_output TEXT,
    model           TEXT,
    cost_cents      INTEGER NOT NULL DEFAULT 0,
    input_tokens    INTEGER NOT NULL DEFAULT 0,
    output_tokens   INTEGER NOT NULL DEFAULT 0,
    created_at      TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_task_outputs_task_id ON task_outputs(task_id);
"#;

/// V4: Add task_dependencies table for task dependency tracking.
pub const SCHEMA_V4: &str = r#"
CREATE TABLE IF NOT EXISTS task_dependencies (
    task_id         TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    depends_on_task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    PRIMARY KEY (task_id, depends_on_task_id)
);
CREATE INDEX IF NOT EXISTS idx_task_dependencies_task_id ON task_dependencies(task_id);
CREATE INDEX IF NOT EXISTS idx_task_dependencies_depends_on ON task_dependencies(depends_on_task_id);
"#;

/// V5: Add queued_at column to tasks and priority index for queue ordering.
pub const SCHEMA_V5: &str = r#"
ALTER TABLE tasks ADD COLUMN queued_at TEXT;
CREATE INDEX IF NOT EXISTS idx_tasks_priority_queued ON tasks(priority, queued_at);
"#;

/// V6: Add webhook_deliveries table for webhook event tracking.
pub const SCHEMA_V6: &str = r#"
CREATE TABLE IF NOT EXISTS webhook_deliveries (
    id              TEXT PRIMARY KEY,
    webhook_name    TEXT NOT NULL,
    url             TEXT NOT NULL,
    event_type      TEXT NOT NULL,
    payload         TEXT,
    status_code     INTEGER,
    success         INTEGER NOT NULL DEFAULT 0,
    error           TEXT,
    delivered_at    TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_webhook_deliveries_at ON webhook_deliveries(delivered_at);
CREATE INDEX IF NOT EXISTS idx_webhook_deliveries_name ON webhook_deliveries(webhook_name);
"#;
