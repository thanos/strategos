# Strategos

A command-line orchestration system for managing AI-assisted software development across multiple projects.

Strategos is the **control plane** for your AI-powered development workflow. It coordinates task routing across multiple AI backends (Claude, Ollama, OpenCode), enforces budget governance, tracks usage, and keeps you in full control of every decision.

## Table of Contents

- [Features](#features)
- [Building from Source](#building-from-source)
- [Quick Start](#quick-start)
- [Configuration Reference](#configuration-reference)
- [Usage Guide](#usage-guide)
  - [Project Management](#project-management)
  - [Submitting Tasks](#submitting-tasks)
  - [Task Queue](#task-queue)
  - [Task Management](#task-management)
  - [Budget and Usage](#budget-and-usage)
  - [Events and History](#events-and-history)
  - [Webhooks](#webhooks)
  - [Templates](#templates)
  - [Batch Processing](#batch-processing)
  - [Dry Run](#dry-run)
- [Tutorials](#tutorials)
  - [Tutorial 1: Your First Project and Task](#tutorial-1-your-first-project-and-task)
  - [Tutorial 2: Budget Governance in Action](#tutorial-2-budget-governance-in-action)
  - [Tutorial 3: Task Queue with Priorities](#tutorial-3-task-queue-with-priorities)
  - [Tutorial 4: Using Templates for Repeated Work](#tutorial-4-using-templates-for-repeated-work)
  - [Tutorial 5: Task Dependencies](#tutorial-5-task-dependencies)
  - [Tutorial 6: Tagging and Filtering Tasks](#tutorial-6-tagging-and-filtering-tasks)
  - [Tutorial 7: Webhooks for Notifications](#tutorial-7-webhooks-for-notifications)
  - [Tutorial 8: Rate Limiting and Circuit Breakers](#tutorial-8-rate-limiting-and-circuit-breakers)
  - [Tutorial 9: Export, Import, and Batch Operations](#tutorial-9-export-import-and-batch-operations)
  - [Tutorial 10: Multi-Project Status Dashboard](#tutorial-10-multi-project-status-dashboard)
- [Architecture Overview](#architecture-overview)
- [Roadmap](#roadmap)

---

## Features

**Implemented (Phases 1-11):**

- Multi-project registry with privacy levels (Public, Private, LocalOnly)
- Provider-neutral adapter layer (Claude, Ollama, OpenCode backends)
- Intelligent task routing by type, capability, budget, and availability
- Budget governance with four modes: Observe, Warn, Govern, Enforce
- Per-backend, per-project, and global budget tracking
- Priority-aware task queue (Critical > High > Normal > Low)
- Exponential backoff with jitter for retries
- Task dependencies with satisfaction checks
- Task tagging and tag-based search
- Webhook event notifications with delivery tracking
- Task templates with placeholder substitution
- Per-backend rate limiting (sliding window)
- Circuit breaker pattern for backend failure tracking
- Concurrent task execution limits (global, per-backend, per-project)
- Batch task submission from TOML files
- Dry-run mode for previewing routing decisions
- Project export/import (JSON)
- Spending trends and usage history
- Full event audit trail with filtering
- Pending action workflow (approve/dismiss)
- SQLite persistence with versioned migrations (V1-V8)

---

## Building from Source

### Prerequisites

- **Rust** 1.85+ (2024 edition) — install via [rustup](https://rustup.rs/)
- **SQLite** is bundled (via `rusqlite` with the `bundled` feature) — no system SQLite needed

### Build

```bash
git clone <repo-url> strategos
cd strategos

# Debug build
cargo build

# Release build (optimized)
cargo build --release

# Run tests (216 tests, 0 failures)
cargo test
```

The binary will be at:
- Debug: `target/debug/strategos`
- Release: `target/release/strategos`

Optionally add to your PATH:

```bash
# From the strategos directory
export PATH="$PWD/target/release:$PATH"

# Or copy to a location on your PATH
cp target/release/strategos ~/.local/bin/
```

---

## Quick Start

### 1. Generate a config file

```bash
strategos init
```

This creates `~/.config/strategos/config.toml` with a sample configuration.

### 2. Edit the config

Open `~/.config/strategos/config.toml` and set your preferences:

```toml
default_backend = "claude"
monthly_budget_dollars = 100.0
budget_mode = "Govern"

[backends.claude]
api_key_env = "ANTHROPIC_API_KEY"
model = "claude-sonnet-4-20250514"

[backends.ollama]
endpoint = "http://localhost:11434"
model = "llama3"

[[projects]]
name = "my-app"
path = "/home/user/projects/my-app"
privacy = "Public"
tags = ["rust", "web"]
```

Make sure your `ANTHROPIC_API_KEY` environment variable is set if using Claude.

### 3. Register a project

```bash
strategos project add my-app /path/to/my-app
```

### 4. Submit a task

```bash
strategos submit --project my-app --task-type review "Review the authentication module for security issues"
```

### 5. Check status

```bash
strategos status
strategos budget
strategos tasks my-app
```

---

## Configuration Reference

The configuration file is TOML format, located by default at `~/.config/strategos/config.toml`. Override with `--config PATH`.

### Top-level Settings

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `default_backend` | string | required | Default AI backend (`"claude"`, `"ollama"`, `"opencode"`) |
| `monthly_budget_dollars` | float | required | Global monthly spending cap in dollars |
| `budget_mode` | string | required | Budget enforcement mode: `"Observe"`, `"Warn"`, `"Govern"`, `"Enforce"` |
| `storage_path` | string | auto | SQLite database path (default: `~/.local/share/strategos/strategos.db`) |
| `log_level` | string | `"info"` | Log level: `error`, `warn`, `info`, `debug`, `trace` |
| `fallback_chain` | list | `["claude", "ollama"]` | Global backend fallback order |

### Backend Configuration

```toml
[backends.claude]
api_key_env = "ANTHROPIC_API_KEY"    # Environment variable holding the API key
model = "claude-sonnet-4-20250514"            # Model to use
monthly_budget_dollars = 80.0         # Optional per-backend budget cap

[backends.ollama]
endpoint = "http://localhost:11434"   # Ollama server URL
model = "llama3"                      # Model name
monthly_budget_dollars = 10.0         # Optional cap

[backends.opencode]
binary_path = "/usr/local/bin/opencode"  # Optional path to binary
```

### Project Configuration

```toml
[[projects]]
name = "my-project"
path = "/home/user/projects/my-project"
default_backend = "claude"            # Optional per-project default
fallback_chain = ["claude", "ollama"] # Optional per-project fallback
monthly_budget_dollars = 20.0         # Optional per-project budget cap
privacy = "Public"                    # Public, Private, or LocalOnly
tags = ["rust", "backend"]            # Optional project tags

[projects.task_overrides]
summarization = "ollama"              # Route specific task types to specific backends
low-cost-drafting = "ollama"
```

### Retry Policy

```toml
[retry_policy]
max_retries = 2              # Number of retry attempts for transient failures
retry_delay_ms = 1000        # Base delay between retries (milliseconds)
backoff_multiplier = 2.0     # Exponential backoff factor
max_delay_ms = 30000         # Maximum delay cap
jitter_fraction = 0.1        # Jitter range (0.0 = none, 1.0 = full)
```

### Webhooks

```toml
[[webhooks]]
name = "slack-notify"
url = "https://hooks.slack.com/services/T.../B.../xxx"
events = ["TaskCompleted", "TaskFailed", "BudgetThresholdReached"]
enabled = true
```

Available event types: `TaskSubmitted`, `TaskCompleted`, `TaskFailed`, `TaskCancelled`, `RoutingDecisionMade`, `BudgetThresholdReached`, `BudgetBlockTriggered`, `BackendDowngradeApplied`, `TaskQueued`, `ActionCreated`, `ActionApproved`, `ActionDismissed`, `WebhookDispatched`.

### Templates

```toml
[[templates]]
name = "code-review"
task_type = "review"
description = "Review {0} for security issues and code quality"
backend = "claude"           # Optional default backend
priority = "high"            # Optional default priority
max_tokens = 4096            # Optional token limit
timeout = 120                # Optional timeout in seconds
max_cost = 500               # Optional max cost in cents
```

Use `{0}`, `{1}`, etc. as placeholders in the description. Arguments are passed as words in the description when using `--template`.

### Rate Limiting

```toml
[[rate_limits]]
backend = "claude"
max_requests_per_minute = 60

[[rate_limits]]
backend = "ollama"
max_requests_per_minute = 120
```

### Concurrency Limits

```toml
[concurrency]
max_concurrent_global = 10          # Total running tasks across all backends
max_concurrent_per_backend = 5      # Optional per-backend cap
max_concurrent_per_project = 3      # Optional per-project cap
```

### Circuit Breaker

```toml
[circuit_breaker]
failure_threshold = 3     # Consecutive failures before tripping
cooldown_secs = 60        # Seconds to wait before retrying a tripped backend
```

---

## Usage Guide

### Project Management

```bash
# Add a project
strategos project add my-app /path/to/my-app
strategos project add secret-research /path/to/research --privacy local-only

# List all projects
strategos project list

# Remove a project (also removes its tasks and usage records)
strategos project remove my-app

# Export project data to JSON (tasks, usage, events)
strategos project export my-app --output my-app-backup.json

# Import project data from JSON
strategos project import my-app-backup.json
```

### Submitting Tasks

```bash
# Basic submission
strategos submit --project my-app --task-type review "Review auth module"

# With backend override
strategos submit --project my-app --task-type summarization --backend ollama "Summarize the README"

# With constraints
strategos submit --project my-app --task-type deep-code-reasoning \
  --max-tokens 8192 \
  --timeout 300 \
  --max-cost 1000 \
  "Analyze the payment processing pipeline for race conditions"

# With priority
strategos submit --project my-app --task-type planning --priority critical \
  "Design the new API versioning strategy"

# With tags
strategos submit --project my-app --task-type review --tag security,auth \
  "Security audit of the login flow"

# With a template
strategos submit --project my-app --template code-review "auth_module.rs"

# Queue instead of running immediately
strategos submit --project my-app --task-type summarization --queue --priority high \
  "Generate API documentation"
```

**Task types** (and CLI aliases):

| Type | CLI values | Default backend |
|------|-----------|-----------------|
| Deep Code Reasoning | `deep-code-reasoning` | Claude |
| Planning | `planning` | Claude |
| Review | `review` | Claude |
| Commit Preparation | `commit-preparation` | Claude |
| Summarization | `summarization`, `summary` | Ollama |
| Backlog Triage | `backlog-triage` | Ollama |
| Low-Cost Drafting | `low-cost-drafting`, `draft` | Ollama |
| Private Local Task | `private-local`, `local` | Ollama |
| Experimental | `experimental`, `experiment` | OpenCode |

### Task Queue

Tasks can be queued for deferred execution instead of running immediately:

```bash
# Queue tasks with different priorities
strategos submit --project my-app --task-type review --queue --priority critical \
  "Urgent security review"
strategos submit --project my-app --task-type summarization --queue --priority low \
  "Update docs"

# View the queue (ordered by priority)
strategos queue list

# Run the next highest-priority task
strategos queue run --project my-app

# Check queue size
strategos queue count
```

Priority ordering: Critical (0) > High (1) > Normal (2) > Low (3). Tasks with the same priority are ordered by queue time (FIFO).

### Task Management

```bash
# List tasks for a project
strategos tasks my-app

# Filter by tag
strategos tasks my-app --tag security

# View task details
strategos task show <task-id-prefix>

# View execution output
strategos task output <task-id-prefix>

# Cancel a pending/running task
strategos task cancel <task-id-prefix>

# Retry a failed task
strategos task retry <task-id-prefix>
strategos task retry <task-id-prefix> --backend ollama  # retry with different backend
```

Task IDs can be specified by prefix (first 8 characters of the UUID).

### Budget and Usage

```bash
# Budget summary (global, per-backend, per-project spend)
strategos budget

# Usage history
strategos usage
strategos usage --project my-app --days 7 --limit 20
strategos usage --backend claude --days 30

# Spending trends
strategos trends --months 3

# Backend health status
strategos health
```

**Budget modes explained:**

| Mode | Behavior |
|------|----------|
| `Observe` | Track spending, log events, never block |
| `Warn` | Log warnings at thresholds (50%, 75%, 90%, 100%), never block |
| `Govern` | Warn at thresholds, suggest downgrades at 90%, require approval at 100% |
| `Enforce` | Block tasks that would exceed budget, auto-downgrade where possible |

### Events and History

```bash
# Recent events (default: 20)
strategos events

# Filter events
strategos events --limit 50
strategos events --type TaskCompleted
strategos events --project my-app
strategos events --task <task-id-prefix>
strategos events --since 2024-01-01 --until 2024-01-31
```

### Webhooks

```bash
# List configured webhooks
strategos webhooks list

# Send a test event to a webhook
strategos webhooks test slack-notify

# View recent webhook deliveries
strategos webhooks deliveries --limit 50
```

### Templates

```bash
# List available templates
strategos templates list

# Show template details
strategos templates show code-review

# Use a template
strategos submit --project my-app --template code-review "auth_module.rs"
```

### Batch Processing

Create a TOML file with multiple tasks:

```toml
# batch.toml
[[tasks]]
project = "my-app"
task_type = "review"
description = "Review authentication module"

[[tasks]]
project = "my-app"
task_type = "summarization"
description = "Summarize the API layer"
backend = "ollama"

[[tasks]]
project = "my-app"
task_type = "planning"
description = "Plan the v2 migration"
priority = "high"
```

Run:

```bash
strategos batch batch.toml
```

### Dry Run

Preview routing decisions without executing:

```bash
strategos dry-run --project my-app --task-type review "Review the auth module"
```

This shows which backend would be selected and why, without submitting the task.

### Pending Actions

When budget governance requires approval, actions are queued:

```bash
# List pending actions
strategos actions list

# List all actions (including resolved)
strategos actions list --all --limit 50

# View action details
strategos actions show <action-id-prefix>

# Approve an action (allows the task to proceed)
strategos actions approve <action-id-prefix>

# Dismiss an action
strategos actions dismiss <action-id-prefix>
```

### Show Configuration

```bash
strategos config
```

---

## Tutorials

### Tutorial 1: Your First Project and Task

This tutorial walks through setting up Strategos from scratch and submitting your first task.

```bash
# Step 1: Initialize config
strategos init
# Output: Config created at /home/user/.config/strategos/config.toml

# Step 2: Register a project
strategos project add my-app ~/projects/my-app
# Output: Project 'my-app' added (a1b2c3d4)

# Step 3: Verify it's registered
strategos project list
# Output:
# NAME        PATH                        PRIVACY
# my-app      /home/user/projects/my-app  Public

# Step 4: Submit a summarization task (routes to Ollama by default)
strategos submit --project my-app --task-type summarization "Summarize the project README"
# Output:
# Submitting Summarization task to project 'my-app'...
# Routed to: ollama (reason: TaskTypeDefault)
# Task completed (cost: $0.00)
# Output: [summary text...]

# Step 5: Check the task list
strategos tasks my-app
# Output:
# STATUS       TYPE                 PRIORITY     TAGS                 DESCRIPTION
# ------------------------------------------------------------------------------------------
# Completed    Summarization        Normal       -                    Summarize the project README

# Step 6: View events
strategos events --limit 5
```

### Tutorial 2: Budget Governance in Action

This tutorial shows how budget modes affect task execution.

```toml
# In config.toml, set:
monthly_budget_dollars = 50.0
budget_mode = "Govern"

[backends.claude]
api_key_env = "ANTHROPIC_API_KEY"
model = "claude-sonnet-4-20250514"
monthly_budget_dollars = 40.0
```

```bash
# Submit tasks and watch budget tracking
strategos submit --project my-app --task-type review "Review auth module"
# Output: Routed to: claude (reason: TaskTypeDefault)

# Check budget after a few tasks
strategos budget
# Output:
# Global: $3.45 / $50.00 (6.9%)
# Backend: claude $3.45 / $40.00 (8.6%)
# Project: my-app $3.45

# Check spending trends
strategos trends --months 1

# When budget reaches 90%, Govern mode suggests downgrade:
# "Budget pressure: recommending downgrade from claude to ollama"

# When budget reaches 100%, Govern mode requires approval:
# "Budget approval required — use `strategos actions approve <id>` to proceed"
strategos actions list
strategos actions approve <action-id>
```

### Tutorial 3: Task Queue with Priorities

Queuing lets you batch work and execute in priority order.

```bash
# Queue several tasks with different priorities
strategos submit --project my-app --task-type review --queue --priority critical \
  "Security audit of payment handler"

strategos submit --project my-app --task-type summarization --queue --priority low \
  "Update changelog"

strategos submit --project my-app --task-type planning --queue --priority high \
  "Plan API versioning"

strategos submit --project my-app --task-type review --queue --priority normal \
  "Code review for PR #42"

# Check the queue — ordered by priority
strategos queue list
# Output (Critical first, then High, Normal, Low):
# PRIORITY     TYPE                 DESCRIPTION
# Critical     Review               Security audit of payment handler
# High         Planning             Plan API versioning
# Normal       Review               Code review for PR #42
# Low          Summarization        Update changelog

# See count
strategos queue count
# Output: 4 tasks queued

# Run the highest-priority task
strategos queue run --project my-app
# Runs "Security audit of payment handler" (Critical priority)

strategos queue count
# Output: 3 tasks queued
```

### Tutorial 4: Using Templates for Repeated Work

Templates save you from retyping common task configurations.

```toml
# In config.toml:
[[templates]]
name = "security-review"
task_type = "review"
description = "Security review of {0}: check for injection, auth bypass, and data exposure"
backend = "claude"
priority = "high"
max_tokens = 8192

[[templates]]
name = "quick-summary"
task_type = "summarization"
description = "Generate a concise summary of {0}"
max_tokens = 2048
```

```bash
# List available templates
strategos templates list
# Output:
# NAME              TASK TYPE       BACKEND
# security-review   review          claude
# quick-summary     summarization   -

# Show details
strategos templates show security-review

# Use a template — the argument replaces {0}
strategos submit --project my-app --template security-review "auth_controller.rs"
# Submits: "Security review of auth_controller.rs: check for injection, auth bypass, and data exposure"
# Uses: claude backend, high priority, 8192 max tokens

strategos submit --project my-app --template quick-summary "the payment module"
# Submits: "Generate a concise summary of the payment module"
```

### Tutorial 5: Task Dependencies

You can make tasks depend on other tasks completing first.

```bash
# Submit a planning task first
strategos submit --project my-app --task-type planning "Design the new caching layer"
# Output: Task a1b2c3d4 completed

# Submit a follow-up that depends on the planning task
strategos submit --project my-app --task-type deep-code-reasoning \
  --depends-on a1b2c3d4 \
  "Implement the caching layer based on the design"
# If a1b2c3d4 is completed → task proceeds
# If a1b2c3d4 is NOT completed → error: "Cannot submit: dependencies not all completed"

# Multiple dependencies (comma-separated)
strategos submit --project my-app --task-type review \
  --depends-on a1b2c3d4,e5f6g7h8 \
  "Review the implementation against the design"
```

### Tutorial 6: Tagging and Filtering Tasks

Tags help organize and filter tasks across your projects.

```bash
# Submit tasks with tags
strategos submit --project my-app --task-type review --tag security,auth \
  "Review login flow"

strategos submit --project my-app --task-type review --tag security,api \
  "Review API endpoints"

strategos submit --project my-app --task-type planning --tag api \
  "Plan API v2"

# List all tasks
strategos tasks my-app
# Shows all tasks with their tags in a TAGS column

# Filter by tag
strategos tasks my-app --tag security
# Shows only the two tasks tagged "security"

strategos tasks my-app --tag api
# Shows the API review and API v2 planning tasks
```

### Tutorial 7: Webhooks for Notifications

Get notified when tasks complete or budgets are hit.

```toml
# In config.toml:
[[webhooks]]
name = "slack-alerts"
url = "https://hooks.slack.com/services/YOUR/WEBHOOK/URL"
events = ["TaskCompleted", "TaskFailed", "BudgetThresholdReached"]
enabled = true

[[webhooks]]
name = "all-events"
url = "https://example.com/strategos-events"
# No events filter = receives ALL events
enabled = true
```

```bash
# Verify webhook configuration
strategos webhooks list
# Output:
# NAME           URL                                        EVENTS                ENABLED
# slack-alerts   https://hooks.slack.com/services/YOUR...   TaskCompleted, ...    true
# all-events     https://example.com/strategos-events       *                     true

# Test a webhook
strategos webhooks test slack-alerts

# After running some tasks, check delivery history
strategos webhooks deliveries --limit 10
# Output:
# WEBHOOK        EVENT            STATUS  DELIVERED AT
# slack-alerts   TaskCompleted    200     2024-03-13T10:30:00Z
# all-events     TaskSubmitted    200     2024-03-13T10:29:58Z
```

### Tutorial 8: Rate Limiting and Circuit Breakers

Protect your backends from overload and handle failures gracefully.

```toml
# In config.toml:

# Limit Claude to 30 requests per minute
[[rate_limits]]
backend = "claude"
max_requests_per_minute = 30

# Trip circuit breaker after 3 consecutive failures, wait 60s before retry
[circuit_breaker]
failure_threshold = 3
cooldown_secs = 60

# Limit concurrent execution
[concurrency]
max_concurrent_global = 10
max_concurrent_per_backend = 5
max_concurrent_per_project = 3
```

```bash
# Rate limiting in action:
# If you submit 31 tasks to Claude within a minute, the 31st will fail with:
# "rate limit exceeded for claude (30/30 per minute)"

# Circuit breaker in action:
# If Claude fails 3 times in a row, the circuit breaker trips:
# "circuit breaker open for claude (3 consecutive failures)"
# After 60 seconds of cooldown, the next request will be allowed through.
# A successful request resets the failure counter.

# Concurrency limits:
# If 10 tasks are already running globally, new submissions will fail:
# "global concurrency limit reached (10/10)"
# Same applies per-backend and per-project if configured.
```

### Tutorial 9: Export, Import, and Batch Operations

#### Project Export/Import

```bash
# Export all data for a project
strategos project export my-app --output my-app-data.json

# The JSON includes project metadata, tasks, usage records, and events.
# Import into a fresh Strategos instance:
strategos project import my-app-data.json
```

#### Batch Task Submission

Create a `batch.toml` file:

```toml
[[tasks]]
project = "my-app"
task_type = "review"
description = "Review authentication module"
priority = "high"
tags = ["security"]

[[tasks]]
project = "my-app"
task_type = "review"
description = "Review database queries for SQL injection"
priority = "critical"
tags = ["security", "database"]

[[tasks]]
project = "my-app"
task_type = "summarization"
description = "Generate API documentation"
backend = "ollama"

[[tasks]]
project = "my-app"
task_type = "planning"
description = "Plan the migration to async handlers"
```

```bash
strategos batch batch.toml
# Submits all 4 tasks sequentially, showing routing and results for each
```

### Tutorial 10: Multi-Project Status Dashboard

Manage many projects from one place.

```bash
# Register multiple projects
strategos project add frontend ~/projects/frontend
strategos project add backend ~/projects/backend --privacy private
strategos project add ml-pipeline ~/projects/ml-pipeline --privacy local-only

# Submit tasks across projects
strategos submit --project frontend --task-type review "Review React components"
strategos submit --project backend --task-type deep-code-reasoning "Analyze API latency"
strategos submit --project ml-pipeline --task-type summarization --backend ollama \
  "Summarize training results"

# Status overview — shows all projects at a glance
strategos status
# Output:
# PROJECT          PENDING  QUEUED   RUNNING  COMPLETED  FAILED   SPEND
# frontend         0        0        0        1          0        $0.15
# backend          0        0        0        1          0        $1.50
# ml-pipeline      0        0        0        1          0        $0.00
# ─────────────────────────────────────────────────────────────────────
# Global budget: $1.65 / $100.00 (1.7%)

# Check health of all backends
strategos health
# Output:
# BACKEND    STATUS     DETAILS
# claude     Healthy    -
# ollama     Healthy    -
# opencode   Unavailable  not configured

# Dry-run to preview routing without executing
strategos dry-run --project frontend --task-type planning "Redesign the navbar"
# Output:
# Would route to: claude
# Reason: TaskTypeDefault
# Fallback applied: false
# Budget downgrade: false
```

---

## Architecture Overview

```
┌──────────────────────────────────────────────────────┐
│                    CLI (clap)                         │
│  submit, tasks, budget, events, queue, webhooks...   │
└───────────────────────┬──────────────────────────────┘
                        │
┌───────────────────────▼──────────────────────────────┐
│                 Orchestrator                          │
│  Task lifecycle, retry, concurrency, circuit breaker │
└──┬────────────┬───────────────┬──────────────────────┘
   │            │               │
   ▼            ▼               ▼
┌──────┐  ┌──────────┐  ┌────────────┐
│Routing│  │  Budget  │  │  Adapter   │
│Engine │  │ Governor │  │ Registry   │
└──┬───┘  └────┬─────┘  └──┬───┬───┬─┘
   │           │            │   │   │
   │           │         ┌──▼┐ ┌▼┐ ┌▼──────┐
   │           │         │ C │ │O│ │OpenCode│
   │           │         │ l │ │l│ │(stub)  │
   │           │         │ a │ │l│ └────────┘
   │           │         │ u │ │a│
   │           │         │ d │ │m│
   │           │         │ e │ │a│
   │           │         └───┘ └─┘
   │           │
┌──▼───────────▼───────────────────────────────────────┐
│                   SQLite Storage                      │
│  projects, tasks, usage, events, routing_history,    │
│  pending_actions, task_outputs, task_dependencies,    │
│  webhook_deliveries, rate_limit_log, circuit_breaker  │
└──────────────────────────────────────────────────────┘
```

**Key design principles:**

- **Explicitness over magic** — Every routing decision includes a reason. Every budget action is visible.
- **Provider-neutral** — The adapter trait abstracts all backends. New backends require only implementing the trait.
- **User control** — No hidden commits, no opaque automation. The human approves, the tool executes.
- **Fixed-precision money** — All costs tracked in integer cents, never floating-point.
- **Event-driven audit trail** — Every significant action emits an event for full traceability.

---

## Roadmap

**Completed:**
- Phase 1-3: Domain models, adapter layer, routing engine, budget governor
- Phase 4: CLI, config hydration, validation
- Phase 5: Config hydration, validation, budget approval workflow
- Phase 6: Execution context, cost estimation, usage history, backend health
- Phase 7: Task output persistence, routing health gate, task cancellation, spending trends
- Phase 8: Full test coverage (162 tests), warnings cleanup
- Phase 9: Task dependencies, export/import, dry-run routing, event filtering
- Phase 10: Priority queuing, retry backoff, webhooks, task templates
- Phase 11: Task tagging, rate limiting, circuit breaker, concurrent limits

**Planned (pending approval):**
- Real HTTP webhook delivery (currently simulated)
- TUI dashboard with live project status
- Git integration (auto-commit suggestions, PR creation)
- Session resume for long-running tasks
- Plugin system for custom adapters
- Multi-user support and access control

---

## License

[TBD]
