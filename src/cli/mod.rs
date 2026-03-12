use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use chrono::Utc;
use clap::{Parser, Subcommand};

use crate::adapters::claude::{ClaudeAdapter, ClaudeConfig};
use crate::adapters::ollama::{OllamaAdapter, OllamaConfig};
use crate::adapters::opencode::{OpenCodeAdapter, OpenCodeConfig};
use crate::adapters::traits::AdapterRegistry;
use crate::budget::governor::{BudgetConfig, BudgetGovernor, InMemoryUsageStore};
use crate::config::GlobalConfig;
use crate::models::policy::{PendingAction, PendingActionType};
use crate::models::project::Project;
use crate::models::task::Task;
use crate::models::{ActionId, BackendId, MoneyAmount, PrivacyLevel, TaskId, TaskType};
use crate::orchestrator::service::Orchestrator;
use crate::routing::engine::{ProjectRoutingConfig, RoutingEngine};
use crate::routing::policy::RoutingPolicy;
use crate::storage::sqlite::{SqliteStorage, ThreadSafeStorage};

#[derive(Parser)]
#[command(name = "strategos", version, about = "AI-assisted development orchestrator")]
pub struct Cli {
    /// Path to config file
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Initialize configuration file
    Init,

    /// Manage projects
    #[command(subcommand)]
    Project(ProjectCommands),

    /// Submit a task for execution
    Submit {
        /// Project name
        #[arg(long)]
        project: String,
        /// Task type
        #[arg(long, value_parser = parse_task_type)]
        task_type: TaskType,
        /// Task description / prompt
        description: Vec<String>,
        /// Override backend selection
        #[arg(long)]
        backend: Option<String>,
    },

    /// Show budget status
    Budget,

    /// Show recent events
    Events {
        /// Number of events to show
        #[arg(long, default_value = "20")]
        limit: usize,
    },

    /// List tasks for a project
    Tasks {
        /// Project name
        project: String,
    },

    /// Show or retry a task
    #[command(subcommand)]
    Task(TaskCommands),

    /// Manage pending actions (review requests, commit suggestions, approvals)
    #[command(subcommand)]
    Actions(ActionCommands),

    /// Show multi-project status overview
    Status,

    /// Prepare a commit message for a project
    PrepareCommit {
        /// Project name
        #[arg(long)]
        project: String,
        /// Override backend selection
        #[arg(long)]
        backend: Option<String>,
    },

    /// Submit code for review
    Review {
        /// Project name
        #[arg(long)]
        project: String,
        /// Files to include in review context
        files: Vec<String>,
        /// Override backend selection
        #[arg(long)]
        backend: Option<String>,
    },

    /// Show current configuration
    Config,
}

#[derive(Subcommand)]
pub enum ProjectCommands {
    /// Add a project
    Add {
        /// Project name
        name: String,
        /// Project path
        path: PathBuf,
        /// Privacy level: public, private, local-only
        #[arg(long, default_value = "public")]
        privacy: String,
    },
    /// List all projects
    List,
    /// Remove a project
    Remove {
        /// Project name
        name: String,
    },
}

#[derive(Subcommand)]
pub enum ActionCommands {
    /// List pending actions
    List {
        /// Show all actions (not just pending)
        #[arg(long)]
        all: bool,
        /// Number of actions to show
        #[arg(long, default_value = "20")]
        limit: usize,
    },
    /// Show action details
    Show {
        /// Action ID (UUID prefix)
        id: String,
    },
    /// Approve a pending action
    Approve {
        /// Action ID (UUID prefix)
        id: String,
    },
    /// Dismiss a pending action
    Dismiss {
        /// Action ID (UUID prefix)
        id: String,
    },
}

#[derive(Subcommand)]
pub enum TaskCommands {
    /// Show detailed task information
    Show {
        /// Task ID (UUID prefix)
        id: String,
    },
    /// Retry a failed task
    Retry {
        /// Task ID (UUID prefix)
        id: String,
        /// Override backend selection
        #[arg(long)]
        backend: Option<String>,
    },
}

fn parse_task_type(s: &str) -> Result<TaskType, String> {
    match s.to_lowercase().as_str() {
        "deep-code-reasoning" | "deepcodereasoning" | "deep_code_reasoning" => {
            Ok(TaskType::DeepCodeReasoning)
        }
        "planning" => Ok(TaskType::Planning),
        "review" => Ok(TaskType::Review),
        "commit-preparation" | "commitpreparation" | "commit_preparation" => {
            Ok(TaskType::CommitPreparation)
        }
        "summarization" | "summary" => Ok(TaskType::Summarization),
        "backlog-triage" | "backlogtriage" | "backlog_triage" => Ok(TaskType::BacklogTriage),
        "low-cost-drafting" | "lowcostdrafting" | "low_cost_drafting" | "draft" => {
            Ok(TaskType::LowCostDrafting)
        }
        "private-local" | "privatelocaltask" | "private_local_task" | "local" => {
            Ok(TaskType::PrivateLocalTask)
        }
        "experimental" | "experiment" => Ok(TaskType::Experimental),
        _ => Err(format!("unknown task type: '{}'. Valid types: planning, review, summarization, deep-code-reasoning, commit-preparation, backlog-triage, low-cost-drafting, private-local, experimental", s)),
    }
}

fn parse_privacy(s: &str) -> Result<PrivacyLevel> {
    match s.to_lowercase().as_str() {
        "public" => Ok(PrivacyLevel::Public),
        "private" => Ok(PrivacyLevel::Private),
        "local-only" | "localonly" | "local_only" => Ok(PrivacyLevel::LocalOnly),
        _ => anyhow::bail!("unknown privacy level: '{}'. Valid: public, private, local-only", s),
    }
}

/// Parsed CLI state including the resolved config and config path.
pub struct ParsedCli {
    pub config_path: PathBuf,
    pub command: Commands,
}

/// Parse CLI args and load config. Called from main before tracing init.
pub fn parse_config() -> Result<(GlobalConfig, ParsedCli)> {
    let cli = Cli::parse();
    let config_path = cli.config.unwrap_or_else(GlobalConfig::default_path);
    let config = if config_path.exists() {
        GlobalConfig::load(&config_path)?
    } else {
        GlobalConfig::sample()
    };
    Ok((config, ParsedCli { config_path, command: cli.command }))
}

/// Run the CLI with pre-loaded config. Called from main after tracing init.
pub async fn run_with(cli: ParsedCli, config: GlobalConfig) -> Result<()> {
    match cli.command {
        Commands::Init => cmd_init(&cli.config_path, &config),
        Commands::Project(sub) => cmd_project(sub, &config),
        Commands::Submit {
            project,
            task_type,
            description,
            backend,
        } => {
            cmd_submit(&config, &project, task_type, &description.join(" "), backend).await
        }
        Commands::Budget => cmd_budget(&config),
        Commands::Events { limit } => cmd_events(&config, limit),
        Commands::Tasks { project } => cmd_tasks(&config, &project),
        Commands::Task(sub) => cmd_task(sub, &config).await,
        Commands::Actions(sub) => cmd_actions(sub, &config),
        Commands::Status => cmd_status(&config),
        Commands::PrepareCommit { project, backend } => {
            cmd_prepare_commit(&config, &project, backend).await
        }
        Commands::Review {
            project,
            files,
            backend,
        } => cmd_review(&config, &project, &files, backend).await,
        Commands::Config => cmd_config(&config, &cli.config_path),
    }
}

// ---------------------------------------------------------------------------
// Command implementations
// ---------------------------------------------------------------------------

fn cmd_init(config_path: &PathBuf, _config: &GlobalConfig) -> Result<()> {
    if config_path.exists() {
        println!("Config already exists at {}", config_path.display());
        return Ok(());
    }
    let config = GlobalConfig::sample();
    config.save(config_path)?;
    println!("Created config at {}", config_path.display());
    println!("Edit this file to configure your backends and projects.");
    Ok(())
}

fn cmd_project(sub: ProjectCommands, config: &GlobalConfig) -> Result<()> {
    let storage = open_storage(config)?;

    match sub {
        ProjectCommands::Add {
            name,
            path,
            privacy,
        } => {
            let privacy = parse_privacy(&privacy)?;
            let mut project = Project::new(&name, &path);
            project.privacy = privacy;
            storage.insert_project(&project)?;
            println!("Added project '{}' at {}", name, path.display());
        }
        ProjectCommands::List => {
            let projects = storage.list_projects()?;
            if projects.is_empty() {
                println!("No projects registered.");
            } else {
                println!("{:<20} {:<12} {}", "NAME", "PRIVACY", "PATH");
                println!("{}", "-".repeat(60));
                for p in &projects {
                    println!("{:<20} {:<12} {}", p.name, format!("{:?}", p.privacy), p.path.display());
                }
                println!("\n{} project(s)", projects.len());
            }
        }
        ProjectCommands::Remove { name } => {
            let project = storage
                .get_project_by_name(&name)?
                .ok_or_else(|| anyhow::anyhow!("project '{}' not found", name))?;
            storage.delete_project(&project.id)?;
            println!("Removed project '{}'", name);
        }
    }

    Ok(())
}

async fn cmd_submit(
    config: &GlobalConfig,
    project_name: &str,
    task_type: TaskType,
    description: &str,
    backend_override: Option<String>,
) -> Result<()> {
    let storage = Arc::new(open_storage(config)?);
    let project = storage
        .get_project_by_name(project_name)?
        .ok_or_else(|| anyhow::anyhow!("project '{}' not found", project_name))?;

    let orchestrator = build_orchestrator(config, Arc::clone(&storage))?;

    let mut task = Task::new(project.id.clone(), task_type, description);
    if let Some(ref b) = backend_override {
        task.backend_override = Some(BackendId::new(b));
    }

    let project_config = build_project_routing_config(config, &project);

    println!("Submitting {:?} task to project '{}'...", task_type, project_name);

    let result = orchestrator
        .submit_task(task, project_config, MoneyAmount::from_cents(100))
        .await?;

    println!(
        "Routed to: {} (reason: {:?})",
        result.routing_decision.selected_backend, result.routing_decision.reason
    );

    if result.requires_approval {
        println!("\nBudget approval required. Task queued for review.");
        if let Some(ref action_id) = result.pending_action_id {
            println!("Approve with: strategos actions approve {}", &action_id.0.to_string()[..8]);
        }
        return Ok(());
    }

    if result.routing_decision.budget_downgrade_applied {
        println!("  (budget downgrade applied)");
    }
    if result.routing_decision.fallback_applied {
        println!("  (fallback applied)");
    }

    match result.execution_output {
        Some(output) => {
            println!("\n--- Output ---");
            println!("{}", output);
        }
        None => {
            println!("Task submitted (backend returned no immediate output — skeleton adapter).");
        }
    }

    if let Some(usage) = result.usage {
        println!(
            "\nUsage: {} input + {} output tokens, cost: {}",
            usage.input_tokens, usage.output_tokens, usage.cost
        );
    }

    Ok(())
}

fn cmd_budget(config: &GlobalConfig) -> Result<()> {
    let storage = Arc::new(open_storage(config)?);
    let orchestrator = build_orchestrator(config, storage)?;

    let year_month = Utc::now().format("%Y-%m").to_string();
    let global_limit = MoneyAmount::from_dollars(config.monthly_budget_dollars);

    let summary = orchestrator.budget_summary(global_limit, &year_month)?;

    println!("Budget Status ({})", year_month);
    println!("{}", "=".repeat(40));
    println!(
        "Global: {} / {} ({}%)",
        summary.global_spent,
        summary.global_limit,
        summary.global_spent.percentage_of(summary.global_limit)
    );

    if !summary.backend_spend.is_empty() {
        println!("\nBy Backend:");
        for (backend, spent) in &summary.backend_spend {
            println!("  {}: {}", backend, spent);
        }
    }

    if !summary.project_spend.is_empty() {
        println!("\nBy Project:");
        for (_, name, spent) in &summary.project_spend {
            println!("  {}: {}", name, spent);
        }
    }

    // Forecast
    let forecast = crate::budget::forecast::BudgetForecast::compute(
        summary.global_spent,
        summary.global_limit,
    );
    println!("\nForecast:");
    println!("  Daily burn rate: {}/day", forecast.daily_burn_rate);
    println!("  Projected EOM: {}", forecast.projected_spend);
    if forecast.projected_overspend {
        println!("  WARNING: projected to exceed budget!");
    }
    if let Some(days) = forecast.days_until_exhaustion {
        if days == 0 {
            println!("  Budget EXHAUSTED");
        } else {
            println!("  Days until exhaustion: {}", days);
        }
    }

    Ok(())
}

fn cmd_events(config: &GlobalConfig, limit: usize) -> Result<()> {
    let storage = open_storage(config)?;
    let events = storage.list_events_recent(limit)?;

    if events.is_empty() {
        println!("No events recorded.");
        return Ok(());
    }

    println!("{:<24} {:<28} {}", "TIMESTAMP", "EVENT", "DETAILS");
    println!("{}", "-".repeat(80));
    for event in &events {
        let ts = event.timestamp.format("%Y-%m-%d %H:%M:%S");
        let details = if event.payload.is_null() {
            String::new()
        } else {
            event.payload.to_string()
        };
        let details_short = if details.len() > 40 {
            format!("{}...", &details[..40])
        } else {
            details
        };
        println!("{:<24} {:<28} {}", ts, format!("{:?}", event.event_type), details_short);
    }

    Ok(())
}

fn cmd_tasks(config: &GlobalConfig, project_name: &str) -> Result<()> {
    let storage = open_storage(config)?;
    let project = storage
        .get_project_by_name(project_name)?
        .ok_or_else(|| anyhow::anyhow!("project '{}' not found", project_name))?;

    let tasks = storage.list_tasks_by_project(&project.id)?;

    if tasks.is_empty() {
        println!("No tasks for project '{}'.", project_name);
        return Ok(());
    }

    println!("{:<12} {:<20} {:<12} {}", "STATUS", "TYPE", "PRIORITY", "DESCRIPTION");
    println!("{}", "-".repeat(70));
    for task in &tasks {
        let desc = if task.description.len() > 30 {
            format!("{}...", &task.description[..30])
        } else {
            task.description.clone()
        };
        println!(
            "{:<12} {:<20} {:<12} {}",
            format!("{:?}", task.status),
            format!("{:?}", task.task_type),
            format!("{:?}", task.priority),
            desc
        );
    }

    Ok(())
}

fn cmd_actions(sub: ActionCommands, config: &GlobalConfig) -> Result<()> {
    let storage = Arc::new(open_storage(config)?);
    let orchestrator = build_orchestrator(config, storage)?;

    match sub {
        ActionCommands::List { all, limit } => {
            let actions = if all {
                orchestrator.list_all_actions(limit)?
            } else {
                orchestrator.list_pending_actions()?
            };

            if actions.is_empty() {
                println!("No {} actions.", if all { "" } else { "pending " });
                return Ok(());
            }

            println!(
                "{:<38} {:<18} {:<10} {}",
                "ID", "TYPE", "STATUS", "DESCRIPTION"
            );
            println!("{}", "-".repeat(90));
            for action in &actions {
                let desc = if action.description.len() > 30 {
                    format!("{}...", &action.description[..30])
                } else {
                    action.description.clone()
                };
                println!(
                    "{:<38} {:<18} {:<10} {}",
                    action.id.0,
                    format!("{:?}", action.action_type),
                    format!("{:?}", action.status),
                    desc
                );
            }
            println!("\n{} action(s)", actions.len());
        }
        ActionCommands::Show { id } => {
            let action_id = resolve_action_id(&orchestrator, &id)?;
            let action = orchestrator
                .get_pending_action(&action_id)?
                .ok_or_else(|| anyhow::anyhow!("action not found"))?;

            println!("Action: {}", action.id.0);
            println!("Type:   {:?}", action.action_type);
            println!("Status: {:?}", action.status);
            println!("Project: {}", action.project_id.0);
            if let Some(ref task_id) = action.task_id {
                println!("Task:   {}", task_id.0);
            }
            println!("Created: {}", action.created_at.format("%Y-%m-%d %H:%M:%S"));
            println!("\nDescription:\n  {}", action.description);
            if !action.payload.is_null() {
                println!(
                    "\nPayload:\n{}",
                    serde_json::to_string_pretty(&action.payload)?
                );
            }
        }
        ActionCommands::Approve { id } => {
            let action_id = resolve_action_id(&orchestrator, &id)?;
            orchestrator.approve_action(&action_id)?;
            println!("Action {} approved.", action_id.0);
        }
        ActionCommands::Dismiss { id } => {
            let action_id = resolve_action_id(&orchestrator, &id)?;
            orchestrator.dismiss_action(&action_id)?;
            println!("Action {} dismissed.", action_id.0);
        }
    }

    Ok(())
}

/// Resolve a UUID prefix to a full ActionId by searching pending actions.
fn resolve_action_id(orchestrator: &Orchestrator, prefix: &str) -> Result<ActionId> {
    // Try parsing as a full UUID first
    if let Ok(uuid) = uuid::Uuid::parse_str(prefix) {
        return Ok(ActionId(uuid));
    }

    // Otherwise, search all actions for a prefix match
    let actions = orchestrator.list_all_actions(100)?;
    let matches: Vec<_> = actions
        .iter()
        .filter(|a| a.id.0.to_string().starts_with(prefix))
        .collect();

    match matches.len() {
        0 => anyhow::bail!("no action matching prefix '{}'", prefix),
        1 => Ok(matches[0].id.clone()),
        n => anyhow::bail!(
            "prefix '{}' is ambiguous ({} matches). Provide more characters.",
            prefix,
            n
        ),
    }
}

/// Resolve a UUID prefix to a full TaskId by searching tasks.
fn resolve_task_id(storage: &SqliteStorage, prefix: &str) -> Result<TaskId> {
    // Try parsing as a full UUID first
    if let Ok(uuid) = uuid::Uuid::parse_str(prefix) {
        return Ok(TaskId(uuid));
    }

    // Search across all projects for a prefix match
    let projects = storage.list_projects()?;
    let mut matches = Vec::new();
    for project in &projects {
        let tasks = storage.list_tasks_by_project(&project.id)?;
        for task in tasks {
            if task.id.0.to_string().starts_with(prefix) {
                matches.push(task.id);
            }
        }
    }

    match matches.len() {
        0 => anyhow::bail!("no task matching prefix '{}'", prefix),
        1 => Ok(matches.into_iter().next().unwrap()),
        n => anyhow::bail!(
            "prefix '{}' is ambiguous ({} matches). Provide more characters.",
            prefix,
            n
        ),
    }
}

async fn cmd_task(sub: TaskCommands, config: &GlobalConfig) -> Result<()> {
    let storage = Arc::new(open_storage(config)?);

    match sub {
        TaskCommands::Show { id } => {
            let task_id = resolve_task_id(&storage, &id)?;
            let task = storage
                .get_task(&task_id)?
                .ok_or_else(|| anyhow::anyhow!("task not found"))?;

            println!("Task: {}", task.id.0);
            println!("Project: {}", task.project_id.0);
            println!("Type:    {:?}", task.task_type);
            println!("Status:  {:?}", task.status);
            println!("Priority: {:?}", task.priority);
            if let Some(ref backend) = task.backend_override {
                println!("Backend override: {}", backend);
            }
            println!("Created: {}", task.created_at.format("%Y-%m-%d %H:%M:%S"));
            println!("Updated: {}", task.updated_at.format("%Y-%m-%d %H:%M:%S"));
            println!("\nDescription:\n  {}", task.description);

            // Show routing history
            if let Some(routing) = storage.get_routing_history_for_task(&task_id)? {
                println!("\nRouting:");
                println!("  Backend: {}", routing.selected_backend);
                println!("  Reason: {}", routing.reason);
                if routing.fallback_applied {
                    println!("  Fallback: yes");
                }
                if routing.budget_downgrade_applied {
                    println!("  Budget downgrade: yes");
                }
            }

            // Show linked actions
            let actions = storage.list_actions_for_task(&task_id)?;
            if !actions.is_empty() {
                println!("\nLinked Actions:");
                for action in &actions {
                    println!(
                        "  {} {:?} ({:?}) - {}",
                        &action.id.0.to_string()[..8],
                        action.action_type,
                        action.status,
                        action.description
                    );
                }
            }
        }
        TaskCommands::Retry { id, backend } => {
            let task_id = resolve_task_id(&storage, &id)?;
            let original = storage
                .get_task(&task_id)?
                .ok_or_else(|| anyhow::anyhow!("task not found"))?;

            if original.status != crate::models::task::TaskStatus::Failed {
                anyhow::bail!(
                    "can only retry failed tasks (current status: {:?})",
                    original.status
                );
            }

            // Look up project name
            let project = storage
                .get_project(&original.project_id)?
                .ok_or_else(|| anyhow::anyhow!("project not found for task"))?;

            let orchestrator = build_orchestrator(config, storage)?;

            let mut new_task =
                Task::new(original.project_id.clone(), original.task_type, &original.description);
            if let Some(ref b) = backend {
                new_task.backend_override = Some(BackendId::new(b));
            } else if let Some(ref b) = original.backend_override {
                new_task.backend_override = Some(b.clone());
            }

            let project_config = build_project_routing_config(config, &project);

            println!(
                "Retrying {:?} task (original: {})...",
                new_task.task_type,
                &id
            );

            let result = orchestrator
                .submit_task(new_task, project_config, MoneyAmount::from_cents(100))
                .await?;

            println!("New task: {}", result.task.id.0);
            println!(
                "Routed to: {} (reason: {:?})",
                result.routing_decision.selected_backend, result.routing_decision.reason
            );

            match result.execution_output {
                Some(output) => {
                    println!("\n--- Output ---");
                    println!("{}", output);
                }
                None => {
                    println!("Task submitted (no immediate output).");
                }
            }
        }
    }

    Ok(())
}

fn cmd_status(config: &GlobalConfig) -> Result<()> {
    let storage = Arc::new(open_storage(config)?);
    let orchestrator = build_orchestrator(config, storage)?;

    let entries = orchestrator.project_status_summary()?;

    if entries.is_empty() {
        println!("No projects registered. Use `strategos project add` to get started.");
        return Ok(());
    }

    let year_month = Utc::now().format("%Y-%m").to_string();
    println!("Status Overview ({})", year_month);
    println!("{}", "=".repeat(70));
    println!(
        "{:<20} {:<10} {:<10} {:<10} {:<10} {}",
        "PROJECT", "PENDING", "RUNNING", "DONE", "ACTIONS", "SPEND"
    );
    println!("{}", "-".repeat(70));

    for entry in &entries {
        let pending = entry
            .task_counts
            .iter()
            .find(|(s, _)| *s == crate::models::task::TaskStatus::Pending)
            .map(|(_, c)| *c)
            .unwrap_or(0);
        let running = entry
            .task_counts
            .iter()
            .find(|(s, _)| *s == crate::models::task::TaskStatus::Running)
            .map(|(_, c)| *c)
            .unwrap_or(0);
        let completed = entry
            .task_counts
            .iter()
            .find(|(s, _)| *s == crate::models::task::TaskStatus::Completed)
            .map(|(_, c)| *c)
            .unwrap_or(0);

        println!(
            "{:<20} {:<10} {:<10} {:<10} {:<10} {}",
            entry.name,
            pending,
            running,
            completed,
            entry.pending_actions,
            entry.month_spend
        );
    }

    // Budget summary
    let budget_limit = MoneyAmount::from_dollars(config.monthly_budget_dollars);
    let total_spend: i64 = entries.iter().map(|e| e.month_spend.cents).sum();
    let total = MoneyAmount::from_cents(total_spend);
    println!(
        "\nTotal spend: {} / {} ({}%)",
        total,
        budget_limit,
        total.percentage_of(budget_limit)
    );

    Ok(())
}

async fn cmd_prepare_commit(
    config: &GlobalConfig,
    project_name: &str,
    backend_override: Option<String>,
) -> Result<()> {
    let storage = Arc::new(open_storage(config)?);
    let project = storage
        .get_project_by_name(project_name)?
        .ok_or_else(|| anyhow::anyhow!("project '{}' not found", project_name))?;

    // Capture git diff from the project directory
    let diff_output = std::process::Command::new("git")
        .args(["diff", "--cached"])
        .current_dir(&project.path)
        .output();

    let diff = match diff_output {
        Ok(output) if output.status.success() => {
            let diff_text = String::from_utf8_lossy(&output.stdout).to_string();
            if diff_text.trim().is_empty() {
                // Fall back to unstaged diff
                let unstaged = std::process::Command::new("git")
                    .args(["diff"])
                    .current_dir(&project.path)
                    .output()
                    .ok()
                    .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
                    .unwrap_or_default();
                if unstaged.trim().is_empty() {
                    anyhow::bail!("no staged or unstaged changes in project '{}'", project_name);
                }
                unstaged
            } else {
                diff_text
            }
        }
        _ => anyhow::bail!("failed to run `git diff` in {}", project.path.display()),
    };

    let prompt = format!(
        "Generate a concise commit message for the following changes. \
         Return only the commit message, no explanation.\n\n\
         ```diff\n{}\n```",
        diff
    );

    let orchestrator = build_orchestrator(config, Arc::clone(&storage))?;

    let mut task = Task::new(project.id.clone(), TaskType::CommitPreparation, &prompt);
    if let Some(ref b) = backend_override {
        task.backend_override = Some(BackendId::new(b));
    }

    let project_config = build_project_routing_config(config, &project);

    println!("Preparing commit message for '{}'...", project_name);

    let result = orchestrator
        .submit_task(task, project_config, MoneyAmount::from_cents(50))
        .await?;

    // Queue result as a pending action
    let commit_msg = result.execution_output.unwrap_or_default();
    let action = PendingAction::new(
        PendingActionType::CommitSuggestion,
        project.id.clone(),
        format!("Suggested commit: {}", &commit_msg.chars().take(80).collect::<String>()),
    )
    .with_task(result.task.id.clone())
    .with_payload(serde_json::json!({
        "commit_message": commit_msg,
        "project": project_name,
    }));

    orchestrator.create_action(&action)?;

    println!("Commit suggestion queued as action {}.", action.id.0);
    println!("Review with: strategos actions show {}", &action.id.0.to_string()[..8]);

    if !commit_msg.is_empty() {
        println!("\n--- Suggested Message ---");
        println!("{}", commit_msg);
    }

    Ok(())
}

async fn cmd_review(
    config: &GlobalConfig,
    project_name: &str,
    files: &[String],
    backend_override: Option<String>,
) -> Result<()> {
    let storage = Arc::new(open_storage(config)?);
    let project = storage
        .get_project_by_name(project_name)?
        .ok_or_else(|| anyhow::anyhow!("project '{}' not found", project_name))?;

    let file_context = if files.is_empty() {
        "Review the recent changes in this project.".to_string()
    } else {
        format!("Review the following files: {}", files.join(", "))
    };

    let prompt = format!(
        "Perform a code review for project '{}'. {}\n\
         Focus on: correctness, security, performance, and style.\n\
         Be concise and actionable.",
        project_name, file_context
    );

    let orchestrator = build_orchestrator(config, Arc::clone(&storage))?;

    let mut task = Task::new(project.id.clone(), TaskType::Review, &prompt);
    if let Some(ref b) = backend_override {
        task.backend_override = Some(BackendId::new(b));
    }

    let project_config = build_project_routing_config(config, &project);

    println!("Submitting review for '{}'...", project_name);

    let result = orchestrator
        .submit_task(task, project_config, MoneyAmount::from_cents(100))
        .await?;

    let review_output = result.execution_output.unwrap_or_default();
    let action = PendingAction::new(
        PendingActionType::ReviewRequest,
        project.id.clone(),
        format!("Code review for '{}'", project_name),
    )
    .with_task(result.task.id.clone())
    .with_payload(serde_json::json!({
        "review": review_output,
        "project": project_name,
        "files": files,
    }));

    orchestrator.create_action(&action)?;

    println!("Review queued as action {}.", action.id.0);
    println!("View with: strategos actions show {}", &action.id.0.to_string()[..8]);

    if !review_output.is_empty() {
        println!("\n--- Review ---");
        println!("{}", review_output);
    }

    Ok(())
}

fn cmd_config(config: &GlobalConfig, config_path: &PathBuf) -> Result<()> {
    println!("Config file: {}", config_path.display());
    println!("Storage: {}", config.storage_path().display());
    println!("Default backend: {}", config.default_backend);
    println!("Budget mode: {:?}", config.budget_mode);
    println!("Monthly budget: ${:.2}", config.monthly_budget_dollars);
    if let Some(ref level) = config.log_level {
        println!("Log level: {}", level);
    }
    if let Some(ref chain) = config.fallback_chain {
        let chain_str: Vec<_> = chain.iter().map(|b| b.as_str()).collect();
        println!("Fallback chain: {}", chain_str.join(" -> "));
    }

    println!("\nBackends:");
    if let Some(ref claude) = config.backends.claude {
        print!("  claude: model={}, key_env={}", claude.model, claude.api_key_env);
        if let Some(budget) = claude.monthly_budget_dollars {
            print!(", budget=${:.2}", budget);
        }
        println!();
    }
    if let Some(ref ollama) = config.backends.ollama {
        print!("  ollama: model={}, endpoint={}", ollama.model, ollama.endpoint);
        if let Some(budget) = ollama.monthly_budget_dollars {
            print!(", budget=${:.2}", budget);
        }
        println!();
    }
    if config.backends.opencode.is_some() {
        println!("  opencode: configured");
    }

    if !config.projects.is_empty() {
        println!("\nProjects (from config):");
        for pc in &config.projects {
            print!("  {}: path={}", pc.name, pc.path.display());
            if let Some(ref backend) = pc.default_backend {
                print!(", backend={}", backend);
            }
            if let Some(budget) = pc.monthly_budget_dollars {
                print!(", budget=${:.2}", budget);
            }
            if let Some(ref privacy) = pc.privacy {
                print!(", privacy={:?}", privacy);
            }
            if let Some(ref overrides) = pc.task_overrides {
                if !overrides.is_empty() {
                    let pairs: Vec<_> = overrides.iter().map(|(k, v)| format!("{}={}", k, v)).collect();
                    print!(", task_overrides=[{}]", pairs.join(", "));
                }
            }
            println!();
        }
    }

    // Validation
    let errors = config.validate();
    if errors.is_empty() {
        println!("\nValidation: OK");
    } else {
        println!("\nValidation errors:");
        for err in &errors {
            println!("  - {}", err);
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn open_storage(config: &GlobalConfig) -> Result<SqliteStorage> {
    let path = config.storage_path();
    Ok(SqliteStorage::open(&path)?)
}

/// Build a fully wired Orchestrator from config and shared storage.
fn build_orchestrator(
    config: &GlobalConfig,
    storage: Arc<SqliteStorage>,
) -> Result<Orchestrator> {
    let (registry, governor) = build_runtime(config, &storage)?;
    let routing_engine = RoutingEngine::new(
        build_routing_policy(config),
        Arc::clone(&registry),
        Arc::clone(&governor),
    );
    Ok(Orchestrator::new(registry, routing_engine, governor, storage))
}

/// Build ProjectRoutingConfig from TOML config and stored project data.
fn build_project_routing_config(
    config: &GlobalConfig,
    project: &Project,
) -> ProjectRoutingConfig {
    // Start with stored project data
    let mut routing_config = ProjectRoutingConfig {
        default_backend: project.default_backend.clone(),
        fallback_chain: project.fallback_chain.clone(),
        privacy: project.privacy,
        task_overrides: HashMap::new(),
    };

    // Overlay TOML config if present
    if let Some(pc) = config.find_project(&project.name) {
        if routing_config.default_backend.is_none() {
            routing_config.default_backend = pc.default_backend.clone();
        }
        if routing_config.fallback_chain.is_empty() {
            if let Some(ref chain) = pc.fallback_chain {
                routing_config.fallback_chain = chain.clone();
            }
        }
        if let Some(ref privacy) = pc.privacy {
            routing_config.privacy = *privacy;
        }
        if let Some(ref overrides) = pc.task_overrides {
            for (key, backend) in overrides {
                if let Ok(tt) = parse_task_type(key) {
                    routing_config.task_overrides.insert(tt, backend.clone());
                }
            }
        }
    }

    routing_config
}

/// Build RoutingPolicy from config (with defaults as fallback).
fn build_routing_policy(config: &GlobalConfig) -> RoutingPolicy {
    let mut policy = RoutingPolicy::default();

    // Override global fallback chain from config
    if let Some(ref chain) = config.fallback_chain {
        policy.global_fallback_chain = chain.clone();
    }

    policy
}

fn build_runtime(
    config: &GlobalConfig,
    storage: &SqliteStorage,
) -> Result<(Arc<AdapterRegistry>, Arc<BudgetGovernor>)> {
    let mut registry = AdapterRegistry::new();

    // Register adapters based on config
    if let Some(ref claude_cfg) = config.backends.claude {
        let adapter = ClaudeAdapter::new(ClaudeConfig {
            api_key_env: claude_cfg.api_key_env.clone(),
            model: claude_cfg.model.clone(),
            ..ClaudeConfig::default()
        });
        registry.register(Arc::new(adapter));
    }

    if let Some(ref ollama_cfg) = config.backends.ollama {
        let adapter = OllamaAdapter::new(OllamaConfig {
            endpoint: ollama_cfg.endpoint.clone(),
            model: ollama_cfg.model.clone(),
            ..OllamaConfig::default()
        });
        registry.register(Arc::new(adapter));
    }

    if let Some(ref _opencode_cfg) = config.backends.opencode {
        let adapter = OpenCodeAdapter::new(OpenCodeConfig::default());
        registry.register(Arc::new(adapter));
    }

    // Build budget config
    let mut backend_limits = HashMap::new();
    if let Some(ref claude_cfg) = config.backends.claude {
        if let Some(limit) = claude_cfg.monthly_budget_dollars {
            backend_limits.insert(BackendId::new("claude"), MoneyAmount::from_dollars(limit));
        }
    }
    if let Some(ref ollama_cfg) = config.backends.ollama {
        if let Some(limit) = ollama_cfg.monthly_budget_dollars {
            backend_limits.insert(BackendId::new("ollama"), MoneyAmount::from_dollars(limit));
        }
    }

    // Populate per-project budget limits from config
    let mut project_limits = HashMap::new();
    for pc in &config.projects {
        if let Some(budget) = pc.monthly_budget_dollars {
            // Look up the project in storage to get its ProjectId
            if let Ok(Some(project)) = storage.get_project_by_name(&pc.name) {
                project_limits.insert(project.id, MoneyAmount::from_dollars(budget));
            }
        }
    }

    let mut downgrade_map = HashMap::new();
    downgrade_map.insert(BackendId::new("claude"), BackendId::new("ollama"));
    downgrade_map.insert(BackendId::new("opencode"), BackendId::new("ollama"));

    // Use ThreadSafeStorage for the usage store
    let usage_store: Arc<dyn crate::budget::governor::UsageStore> =
        match ThreadSafeStorage::open(&config.storage_path()) {
            Ok(ts) => Arc::new(ts),
            Err(_) => Arc::new(InMemoryUsageStore::new()),
        };

    let budget_config = BudgetConfig {
        mode: config.budget_mode,
        global_monthly_limit: MoneyAmount::from_dollars(config.monthly_budget_dollars),
        backend_limits,
        project_limits,
        thresholds: vec![50, 75, 90, 100],
        downgrade_map,
    };

    let governor = BudgetGovernor::new(budget_config, usage_store);

    Ok((Arc::new(registry), Arc::new(governor)))
}
