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
    project_id      TEXT NOT NULL REFERENCES projects(id),
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
    task_id       TEXT NOT NULL REFERENCES tasks(id),
    project_id    TEXT NOT NULL REFERENCES projects(id),
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
    project_id  TEXT NOT NULL REFERENCES projects(id),
    task_id     TEXT,
    description TEXT NOT NULL,
    payload     TEXT,
    status      TEXT NOT NULL DEFAULT 'Pending',
    created_at  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS routing_history (
    id                        TEXT PRIMARY KEY,
    task_id                   TEXT NOT NULL REFERENCES tasks(id),
    selected_backend          TEXT NOT NULL,
    reason                    TEXT NOT NULL,
    fallback_applied          INTEGER NOT NULL DEFAULT 0,
    budget_downgrade_applied  INTEGER NOT NULL DEFAULT 0,
    evaluated_backends        TEXT,
    decided_at                TEXT NOT NULL
);
"#;
