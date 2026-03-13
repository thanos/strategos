use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use chrono::Utc;
use clap::{Parser, Subcommand};

use crate::adapters::claude::{ClaudeAdapter, ClaudeConfig};
use crate::adapters::traits::estimate_task_cost;
use crate::adapters::ollama::{OllamaAdapter, OllamaConfig};
use crate::adapters::opencode::{OpenCodeAdapter, OpenCodeConfig};
use crate::adapters::traits::AdapterRegistry;
use crate::budget::governor::{BudgetConfig, BudgetGovernor, InMemoryUsageStore};
use crate::config::GlobalConfig;
use crate::models::event::{Event, EventType};
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
        /// Task type (optional when --template is used)
        #[arg(long, value_parser = parse_task_type)]
        task_type: Option<TaskType>,
        /// Task description / prompt
        description: Vec<String>,
        /// Override backend selection
        #[arg(long)]
        backend: Option<String>,
        /// Maximum output tokens
        #[arg(long)]
        max_tokens: Option<u64>,
        /// Timeout in seconds
        #[arg(long)]
        timeout: Option<u64>,
        /// Maximum cost in cents
        #[arg(long)]
        max_cost: Option<i64>,
        /// Comma-separated task ID prefixes this task depends on
        #[arg(long)]
        depends_on: Option<String>,
        /// Queue the task instead of executing immediately
        #[arg(long)]
        queue: bool,
        /// Task priority (low, normal, high, critical)
        #[arg(long, value_parser = parse_priority)]
        priority: Option<crate::models::Priority>,
        /// Use a task template by name
        #[arg(long)]
        template: Option<String>,
        /// Comma-separated tags for the task
        #[arg(long)]
        tag: Option<String>,
    },

    /// Show budget status
    Budget,

    /// Show recent events
    Events {
        /// Number of events to show
        #[arg(long, default_value = "20")]
        limit: usize,
        /// Filter by event type (e.g. TaskSubmitted, TaskCompleted)
        #[arg(long, name = "type")]
        event_type: Option<String>,
        /// Filter by project name
        #[arg(long)]
        project: Option<String>,
        /// Filter by task ID prefix
        #[arg(long)]
        task: Option<String>,
        /// Show events since this date (YYYY-MM-DD or RFC3339)
        #[arg(long)]
        since: Option<String>,
        /// Show events until this date (YYYY-MM-DD or RFC3339)
        #[arg(long)]
        until: Option<String>,
    },

    /// List tasks for a project
    Tasks {
        /// Project name
        project: String,
        /// Filter tasks by tag
        #[arg(long)]
        tag: Option<String>,
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

    /// Show spending trends over time
    Trends {
        /// Number of months to show
        #[arg(long, default_value = "3")]
        months: u32,
    },

    /// Check backend health status
    Health,

    /// Show usage history
    Usage {
        /// Filter by project name
        #[arg(long)]
        project: Option<String>,
        /// Filter by backend
        #[arg(long)]
        backend: Option<String>,
        /// Show records from the last N days
        #[arg(long, default_value = "30")]
        days: u32,
        /// Maximum number of records
        #[arg(long, default_value = "50")]
        limit: usize,
    },

    /// Submit multiple tasks from a TOML batch file
    Batch {
        /// Path to batch TOML file
        file: PathBuf,
    },

    /// Preview routing decision without executing
    DryRun {
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

    /// Manage the task queue
    #[command(subcommand)]
    Queue(QueueCommands),

    /// Manage webhooks
    #[command(subcommand)]
    Webhooks(WebhookCommands),

    /// Manage task templates
    #[command(subcommand)]
    Templates(TemplateCommands),

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
    /// Export project data to JSON
    Export {
        /// Project name
        name: String,
        /// Output file path
        #[arg(long)]
        output: PathBuf,
    },
    /// Import project data from JSON
    Import {
        /// Path to JSON file
        file: PathBuf,
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
    /// Show task execution output
    Output {
        /// Task ID (UUID prefix)
        id: String,
    },
    /// Cancel a pending or running task
    Cancel {
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

#[derive(Subcommand)]
pub enum QueueCommands {
    /// List queued tasks
    List,
    /// Run the next queued task
    Run {
        /// Project name (for routing config)
        #[arg(long)]
        project: String,
    },
    /// Show queue count
    Count,
}

#[derive(Subcommand)]
pub enum WebhookCommands {
    /// List configured webhooks
    List,
    /// Test a webhook by sending a test event
    Test {
        /// Webhook name
        name: String,
    },
    /// Show recent webhook deliveries
    Deliveries {
        /// Number of deliveries to show
        #[arg(long, default_value = "20")]
        limit: usize,
    },
}

#[derive(Subcommand)]
pub enum TemplateCommands {
    /// List available templates
    List,
    /// Show template details
    Show {
        /// Template name
        name: String,
    },
}

fn parse_priority(s: &str) -> Result<crate::models::Priority, String> {
    match s.to_lowercase().as_str() {
        "low" => Ok(crate::models::Priority::Low),
        "normal" => Ok(crate::models::Priority::Normal),
        "high" => Ok(crate::models::Priority::High),
        "critical" => Ok(crate::models::Priority::Critical),
        _ => Err(format!("unknown priority: '{}'. Valid: low, normal, high, critical", s)),
    }
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

/// Sync projects defined in TOML config to storage.
/// Adds new projects and updates paths/privacy for existing ones.
/// Does NOT delete projects that are only in storage (user may have added via CLI).
pub fn sync_projects_from_config(config: &GlobalConfig, storage: &SqliteStorage) {
    for pc in &config.projects {
        match storage.get_project_by_name(&pc.name) {
            Ok(Some(mut existing)) => {
                // Update path and privacy if they differ
                let mut changed = false;
                if existing.path != pc.path {
                    existing.path = pc.path.clone();
                    changed = true;
                }
                if let Some(privacy) = pc.privacy {
                    if existing.privacy != privacy {
                        existing.privacy = privacy;
                        changed = true;
                    }
                }
                if changed {
                    let _ = storage.update_project(&existing);
                    tracing::info!(project = %pc.name, "synced project from config");
                }
            }
            Ok(None) => {
                // Project not in storage — add it
                let mut project = crate::models::project::Project::new(&pc.name, &pc.path);
                if let Some(privacy) = pc.privacy {
                    project.privacy = privacy;
                }
                if let Some(ref tags) = pc.tags {
                    project.tags = tags.clone();
                }
                if let Some(ref backend) = pc.default_backend {
                    project.default_backend = Some(backend.clone());
                }
                if let Some(ref chain) = pc.fallback_chain {
                    project.fallback_chain = chain.clone();
                }
                match storage.insert_project(&project) {
                    Ok(()) => tracing::info!(project = %pc.name, "added project from config"),
                    Err(e) => tracing::warn!(project = %pc.name, error = %e, "failed to sync project"),
                }
            }
            Err(e) => {
                tracing::warn!(project = %pc.name, error = %e, "failed to look up project during sync");
            }
        }
    }
}

/// Run the CLI with pre-loaded config. Called from main after tracing init.
pub async fn run_with(cli: ParsedCli, config: GlobalConfig) -> Result<()> {
    // Auto-sync projects from config to storage on every run
    if let Ok(storage) = open_storage(&config) {
        sync_projects_from_config(&config, &storage);
    }
    match cli.command {
        Commands::Init => cmd_init(&cli.config_path, &config),
        Commands::Project(sub) => cmd_project(sub, &config),
        Commands::Submit {
            project,
            task_type,
            description,
            backend,
            max_tokens,
            timeout,
            max_cost,
            depends_on,
            queue,
            priority,
            template,
            tag,
        } => {
            let tags: Vec<String> = tag
                .map(|t| t.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
                .unwrap_or_default();
            cmd_submit(&config, &project, task_type, &description.join(" "), backend, max_tokens, timeout, max_cost, depends_on, queue, priority, template.as_deref(), tags).await
        }
        Commands::Budget => cmd_budget(&config),
        Commands::Events { limit, event_type, project, task, since, until } => {
            cmd_events(&config, limit, event_type, project, task, since, until)
        }
        Commands::Tasks { project, tag } => cmd_tasks(&config, &project, tag.as_deref()),
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
        Commands::Trends { months } => cmd_trends(&config, months),
        Commands::Health => cmd_health(&config).await,
        Commands::Usage {
            project,
            backend,
            days,
            limit,
        } => cmd_usage(&config, project.as_deref(), backend.as_deref(), days, limit),
        Commands::Batch { file } => cmd_batch(&config, &file).await,
        Commands::DryRun {
            project,
            task_type,
            description,
            backend,
        } => cmd_dry_run(&config, &project, task_type, &description.join(" "), backend).await,
        Commands::Queue(sub) => cmd_queue(sub, &config).await,
        Commands::Webhooks(sub) => cmd_webhooks(sub, &config),
        Commands::Templates(sub) => cmd_templates(sub, &config),
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
        ProjectCommands::Export { name, output } => {
            let project = storage
                .get_project_by_name(&name)?
                .ok_or_else(|| anyhow::anyhow!("project '{}' not found", name))?;

            let data = storage.export_project_data(&project.id)?;
            let json = serde_json::to_string_pretty(&data)?;
            std::fs::write(&output, &json)?;
            println!(
                "Exported project '{}': {} tasks, {} usage records, {} actions -> {}",
                name,
                data.tasks.len(),
                data.usage_records.len(),
                data.actions.len(),
                output.display()
            );
        }
        ProjectCommands::Import { file } => {
            let content = std::fs::read_to_string(&file)
                .map_err(|e| anyhow::anyhow!("cannot read '{}': {}", file.display(), e))?;
            let data: crate::storage::sqlite::ProjectExportData = serde_json::from_str(&content)?;
            let result = storage.import_project_data(&data)?;

            println!("Import from {}:", file.display());
            if result.imported_project {
                println!("  Project: imported");
            } else {
                println!("  Project: skipped (already exists)");
            }
            println!(
                "  Tasks:   {} imported, {} skipped",
                result.imported_tasks, result.skipped_tasks
            );
            println!(
                "  Usage:   {} imported, {} skipped",
                result.imported_usage, result.skipped_usage
            );
            println!(
                "  Actions: {} imported, {} skipped",
                result.imported_actions, result.skipped_actions
            );
        }
    }

    Ok(())
}

async fn cmd_submit(
    config: &GlobalConfig,
    project_name: &str,
    task_type_opt: Option<TaskType>,
    description: &str,
    backend_override: Option<String>,
    max_tokens: Option<u64>,
    timeout_secs: Option<u64>,
    max_cost_cents: Option<i64>,
    depends_on: Option<String>,
    queue: bool,
    priority: Option<crate::models::Priority>,
    template_name: Option<&str>,
    tags: Vec<String>,
) -> Result<()> {
    let storage = Arc::new(open_storage(config)?);
    let project = storage
        .get_project_by_name(project_name)?
        .ok_or_else(|| anyhow::anyhow!("project '{}' not found", project_name))?;

    let orchestrator = build_orchestrator(config, Arc::clone(&storage))?;

    // Resolve template if provided
    let (task_type, final_description, tmpl_backend, tmpl_max_tokens, tmpl_timeout, tmpl_max_cost) =
        if let Some(tmpl_name) = template_name {
            let templates = config.templates.as_ref()
                .ok_or_else(|| anyhow::anyhow!("no templates configured"))?;
            let tmpl = templates.iter().find(|t| t.name == tmpl_name)
                .ok_or_else(|| anyhow::anyhow!("template '{}' not found", tmpl_name))?;
            let tt = parse_task_type(&tmpl.task_type)
                .map_err(|e| anyhow::anyhow!("template task_type error: {}", e))?;
            let desc = if description.is_empty() {
                let args: Vec<&str> = Vec::new();
                tmpl.resolve_description(&args)
                    .map_err(|e| anyhow::anyhow!("template error: {}", e))?
            } else {
                let args: Vec<&str> = description.split_whitespace().collect();
                tmpl.resolve_description(&args)
                    .unwrap_or_else(|_| description.to_string())
            };
            let tt_override = task_type_opt.unwrap_or(tt);
            (tt_override, desc, tmpl.backend.clone(), tmpl.max_tokens, tmpl.timeout, tmpl.max_cost)
        } else {
            let tt = task_type_opt
                .ok_or_else(|| anyhow::anyhow!("--task-type is required (or use --template)"))?;
            (tt, description.to_string(), None, None, None, None)
        };

    let effective_backend = backend_override.or(tmpl_backend);
    let effective_max_tokens = max_tokens.or(tmpl_max_tokens);
    let effective_timeout = timeout_secs.or(tmpl_timeout);
    let effective_max_cost = max_cost_cents.or(tmpl_max_cost);

    let mut task = Task::new(project.id.clone(), task_type, &final_description);
    if let Some(ref b) = effective_backend {
        task.backend_override = Some(BackendId::new(b));
    }
    if let Some(p) = priority {
        task.priority = p;
    }
    if !tags.is_empty() {
        task.tags = tags;
    }

    // Resolve and validate dependencies
    let dep_ids = if let Some(ref deps_str) = depends_on {
        let mut ids = Vec::new();
        for prefix in deps_str.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
            let dep_id = resolve_task_id(&storage, prefix)?;
            ids.push(dep_id);
        }
        ids
    } else {
        Vec::new()
    };

    // Store the task first so we can register dependencies
    if !dep_ids.is_empty() {
        // Pre-check: persist task, register deps, then check all completed
        storage.insert_task(&task)?;
        orchestrator.add_task_dependencies(&task.id, &dep_ids)?;

        if !orchestrator.check_dependencies(&task.id)? {
            // Unsatisfied — mark as failed and report
            storage.update_task_status(&task.id, crate::models::task::TaskStatus::Failed)?;
            let dep_strs: Vec<String> = dep_ids.iter().map(|d| d.0.to_string()[..8].to_string()).collect();
            anyhow::bail!(
                "Cannot submit: dependencies not all completed: [{}]",
                dep_strs.join(", ")
            );
        }
    }

    // Queue mode: just queue the task and return
    if queue {
        orchestrator.queue_task(&mut task)?;
        println!("Task {} queued (priority: {:?}, rank: {})", &task.id.0.to_string()[..8], task.priority, task.priority.rank());
        return Ok(());
    }

    let project_config = build_project_routing_config(config, &project);

    println!("Submitting {:?} task to project '{}'...", task_type, project_name);

    let estimated_cost = estimate_task_cost(
        &final_description,
        &config.default_backend,
        config.backends.claude.as_ref().map(|c| c.model.as_str()).unwrap_or("claude-sonnet-4-20250514"),
    );

    let constraints = crate::adapters::traits::ExecutionConstraints {
        max_tokens: effective_max_tokens,
        max_cost_cents: effective_max_cost,
        timeout: effective_timeout.map(std::time::Duration::from_secs),
        ..crate::adapters::traits::ExecutionConstraints::default()
    };

    let result = orchestrator
        .submit_task_with_context(
            task,
            project_config,
            estimated_cost,
            Some(project.path.clone()),
            Vec::new(),
            constraints,
        )
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

fn cmd_events(
    config: &GlobalConfig,
    limit: usize,
    event_type: Option<String>,
    project_name: Option<String>,
    task_prefix: Option<String>,
    since: Option<String>,
    until: Option<String>,
) -> Result<()> {
    let storage = open_storage(config)?;

    let has_filters = event_type.is_some() || project_name.is_some() || task_prefix.is_some() || since.is_some() || until.is_some();

    let events = if has_filters {
        let project_id = if let Some(ref name) = project_name {
            let project = storage
                .get_project_by_name(name)?
                .ok_or_else(|| anyhow::anyhow!("project '{}' not found", name))?;
            Some(project.id)
        } else {
            None
        };

        let task_id = if let Some(ref prefix) = task_prefix {
            Some(resolve_task_id(&storage, prefix)?)
        } else {
            None
        };

        storage.list_events_filtered(
            event_type.as_deref(),
            project_id.as_ref(),
            task_id.as_ref(),
            since.as_deref(),
            until.as_deref(),
            limit,
        )?
    } else {
        storage.list_events_recent(limit)?
    };

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

    println!("\n{} event(s)", events.len());
    Ok(())
}

fn cmd_tasks(config: &GlobalConfig, project_name: &str, tag_filter: Option<&str>) -> Result<()> {
    let storage = open_storage(config)?;
    let project = storage
        .get_project_by_name(project_name)?
        .ok_or_else(|| anyhow::anyhow!("project '{}' not found", project_name))?;

    let tasks = if let Some(tag) = tag_filter {
        storage.search_tasks_by_tag(tag)?
            .into_iter()
            .filter(|t| t.project_id == project.id)
            .collect()
    } else {
        storage.list_tasks_by_project(&project.id)?
    };

    if tasks.is_empty() {
        println!("No tasks for project '{}'.", project_name);
        return Ok(());
    }

    println!("{:<12} {:<20} {:<12} {:<20} {}", "STATUS", "TYPE", "PRIORITY", "TAGS", "DESCRIPTION");
    println!("{}", "-".repeat(90));
    for task in &tasks {
        let desc = if task.description.len() > 30 {
            format!("{}...", &task.description[..30])
        } else {
            task.description.clone()
        };
        let tags_str = if task.tags.is_empty() {
            "-".to_string()
        } else {
            task.tags.join(", ")
        };
        println!(
            "{:<12} {:<20} {:<12} {:<20} {}",
            format!("{:?}", task.status),
            format!("{:?}", task.task_type),
            format!("{:?}", task.priority),
            tags_str,
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

            // Show dependencies
            let deps = storage.get_task_dependencies(&task_id)?;
            if !deps.is_empty() {
                println!("\nDependencies:");
                for dep_id in &deps {
                    let status = storage
                        .get_task(dep_id)?
                        .map(|t| format!("{:?}", t.status))
                        .unwrap_or_else(|| "unknown".to_string());
                    println!("  {} ({})", &dep_id.0.to_string()[..8], status);
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
        TaskCommands::Output { id } => {
            let task_id = resolve_task_id(&storage, &id)?;
            let task = storage
                .get_task(&task_id)?
                .ok_or_else(|| anyhow::anyhow!("task not found"))?;

            println!("Task: {} ({:?})", task.id.0, task.status);

            match storage.get_task_output(&task_id)? {
                Some(output_row) => {
                    println!("Backend: {}", output_row.backend_id);
                    if let Some(ref model) = output_row.model {
                        println!("Model:   {}", model);
                    }
                    println!(
                        "Tokens:  {} input / {} output",
                        output_row.input_tokens, output_row.output_tokens
                    );
                    println!(
                        "Cost:    {}",
                        MoneyAmount::from_cents(output_row.cost_cents)
                    );
                    println!("Created: {}", output_row.created_at);
                    println!("\n--- Output ---");
                    println!("{}", output_row.output);
                    if let Some(ref structured) = output_row.structured_output {
                        println!("\n--- Structured Output ---");
                        if let Ok(value) = serde_json::from_str::<serde_json::Value>(structured) {
                            println!("{}", serde_json::to_string_pretty(&value)?);
                        } else {
                            println!("{}", structured);
                        }
                    }
                }
                None => {
                    println!("No output stored for this task.");
                }
            }
        }
        TaskCommands::Cancel { id } => {
            let task_id = resolve_task_id(&storage, &id)?;
            let orchestrator = build_orchestrator(config, storage)?;

            match orchestrator.cancel_task(&task_id) {
                Ok(()) => println!("Task {} cancelled.", task_id.0),
                Err(crate::orchestrator::service::CancelError::InvalidState(msg)) => {
                    anyhow::bail!("Cannot cancel: {}", msg);
                }
                Err(e) => return Err(e.into()),
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

            let estimated_cost = estimate_task_cost(
                &original.description,
                &config.default_backend,
                config.backends.claude.as_ref().map(|c| c.model.as_str()).unwrap_or("claude-sonnet-4-20250514"),
            );

            let result = orchestrator
                .submit_task_with_context(
                    new_task,
                    project_config,
                    estimated_cost,
                    Some(project.path.clone()),
                    Vec::new(),
                    crate::adapters::traits::ExecutionConstraints::default(),
                )
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

    let estimated_cost = estimate_task_cost(
        &prompt,
        &config.default_backend,
        config.backends.claude.as_ref().map(|c| c.model.as_str()).unwrap_or("claude-sonnet-4-20250514"),
    );

    let result = orchestrator
        .submit_task_with_context(
            task,
            project_config,
            estimated_cost,
            Some(project.path.clone()),
            Vec::new(),
            crate::adapters::traits::ExecutionConstraints::default(),
        )
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

    let context_files: Vec<std::path::PathBuf> = files
        .iter()
        .map(|f| project.path.join(f))
        .collect();

    let estimated_cost = estimate_task_cost(
        &prompt,
        &config.default_backend,
        config.backends.claude.as_ref().map(|c| c.model.as_str()).unwrap_or("claude-sonnet-4-20250514"),
    );

    println!("Submitting review for '{}'...", project_name);

    let result = orchestrator
        .submit_task_with_context(
            task,
            project_config,
            estimated_cost,
            Some(project.path.clone()),
            context_files,
            crate::adapters::traits::ExecutionConstraints::default(),
        )
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

fn cmd_trends(config: &GlobalConfig, months: u32) -> Result<()> {
    let storage = open_storage(config)?;

    let monthly = storage.spend_by_month(months)?;
    let by_backend = storage.spend_by_backend_month(months)?;
    let by_project = storage.spend_by_project_month(months)?;

    if monthly.is_empty() {
        println!("No spending data available.");
        return Ok(());
    }

    println!("Spending Trends (last {} months)", months);
    println!("{}", "=".repeat(50));

    // Monthly totals with month-over-month change
    println!("\n{:<12} {:<15} {}", "MONTH", "SPEND", "CHANGE");
    println!("{}", "-".repeat(40));
    let mut prev_cents: Option<i64> = None;
    // Reverse so oldest is first for change calculation
    let monthly_ordered: Vec<_> = monthly.iter().rev().collect();
    for (ym, amount) in &monthly_ordered {
        let change = match prev_cents {
            Some(prev) if prev > 0 => {
                let pct = ((amount.cents - prev) as f64 / prev as f64) * 100.0;
                if pct > 0.0 {
                    format!("+{:.0}%", pct)
                } else {
                    format!("{:.0}%", pct)
                }
            }
            _ => "—".to_string(),
        };
        println!("{:<12} {:<15} {}", ym, amount, change);
        prev_cents = Some(amount.cents);
    }

    // By backend
    if !by_backend.is_empty() {
        println!("\nBy Backend:");
        println!("{:<12} {:<15} {}", "MONTH", "BACKEND", "SPEND");
        println!("{}", "-".repeat(40));
        for (ym, backend, amount) in &by_backend {
            println!("{:<12} {:<15} {}", ym, backend, amount);
        }
    }

    // By project
    if !by_project.is_empty() {
        println!("\nBy Project:");
        println!("{:<12} {:<38} {}", "MONTH", "PROJECT", "SPEND");
        println!("{}", "-".repeat(55));
        // Resolve project names
        for (ym, project_id, amount) in &by_project {
            let name = storage
                .get_project(project_id)
                .ok()
                .flatten()
                .map(|p| p.name)
                .unwrap_or_else(|| project_id.0.to_string());
            println!("{:<12} {:<38} {}", ym, name, amount);
        }
    }

    Ok(())
}

async fn cmd_health(config: &GlobalConfig) -> Result<()> {
    let storage = Arc::new(open_storage(config)?);
    let (registry, _governor) = build_runtime(config, &storage)?;

    println!("Backend Health Check");
    println!("{}", "=".repeat(50));
    println!("{:<15} {:<35}", "BACKEND", "STATUS");
    println!("{}", "-".repeat(50));

    for backend_id in registry.list() {
        if let Some(adapter) = registry.get(backend_id) {
            let status = adapter.health_check().await;
            println!("{:<15} {}", backend_id, status);
        }
    }

    Ok(())
}

fn cmd_usage(
    config: &GlobalConfig,
    project_name: Option<&str>,
    backend_name: Option<&str>,
    days: u32,
    limit: usize,
) -> Result<()> {
    let storage = open_storage(config)?;

    let project_id = if let Some(name) = project_name {
        let project = storage
            .get_project_by_name(name)?
            .ok_or_else(|| anyhow::anyhow!("project '{}' not found", name))?;
        Some(project.id)
    } else {
        None
    };

    let backend_id = backend_name.map(BackendId::new);

    let since = {
        let cutoff = Utc::now() - chrono::Duration::days(days as i64);
        cutoff.to_rfc3339()
    };

    let records = storage.list_usage_records(
        project_id.as_ref(),
        backend_id.as_ref(),
        Some(&since),
        limit,
    )?;

    if records.is_empty() {
        println!("No usage records found.");
        return Ok(());
    }

    println!(
        "{:<20} {:<10} {:<10} {:<10} {:<10} {}",
        "TIMESTAMP", "BACKEND", "INPUT", "OUTPUT", "COST", "TASK"
    );
    println!("{}", "-".repeat(75));

    let mut total_cost = MoneyAmount::ZERO;
    let mut total_input = 0u64;
    let mut total_output = 0u64;

    for record in &records {
        total_cost = total_cost + record.cost;
        total_input += record.input_tokens;
        total_output += record.output_tokens;

        let ts = record.recorded_at.format("%Y-%m-%d %H:%M");
        let task_short = &record.task_id.0.to_string()[..8];
        println!(
            "{:<20} {:<10} {:<10} {:<10} {:<10} {}",
            ts,
            record.backend_id,
            record.input_tokens,
            record.output_tokens,
            record.cost,
            task_short
        );
    }

    println!("{}", "-".repeat(75));
    println!(
        "{:<20} {:<10} {:<10} {:<10} {}",
        format!("{} record(s)", records.len()),
        "",
        total_input,
        total_output,
        total_cost
    );

    Ok(())
}

/// A task entry in a batch TOML file.
#[derive(Debug, serde::Deserialize)]
struct BatchTaskEntry {
    project: String,
    task_type: String,
    description: String,
    backend: Option<String>,
}

/// Top-level batch file structure.
#[derive(Debug, serde::Deserialize)]
struct BatchFile {
    tasks: Vec<BatchTaskEntry>,
}

async fn cmd_batch(config: &GlobalConfig, file: &PathBuf) -> Result<()> {
    let content = std::fs::read_to_string(file)
        .map_err(|e| anyhow::anyhow!("cannot read batch file '{}': {}", file.display(), e))?;
    let batch: BatchFile = toml::from_str(&content)
        .map_err(|e| anyhow::anyhow!("invalid batch file: {}", e))?;

    if batch.tasks.is_empty() {
        println!("Batch file contains no tasks.");
        return Ok(());
    }

    let storage = Arc::new(open_storage(config)?);
    let orchestrator = build_orchestrator(config, Arc::clone(&storage))?;

    let total = batch.tasks.len();
    let mut succeeded = 0usize;
    let mut failed = 0usize;
    let mut total_cost = MoneyAmount::ZERO;

    println!("Submitting {} task(s) from batch file...\n", total);

    for (i, entry) in batch.tasks.iter().enumerate() {
        let task_type = match parse_task_type(&entry.task_type) {
            Ok(tt) => tt,
            Err(e) => {
                println!("[{}/{}] SKIP: {}", i + 1, total, e);
                failed += 1;
                continue;
            }
        };

        let project = match storage.get_project_by_name(&entry.project)? {
            Some(p) => p,
            None => {
                println!("[{}/{}] SKIP: project '{}' not found", i + 1, total, entry.project);
                failed += 1;
                continue;
            }
        };

        let mut task = Task::new(project.id.clone(), task_type, &entry.description);
        if let Some(ref b) = entry.backend {
            task.backend_override = Some(BackendId::new(b));
        }

        let project_config = build_project_routing_config(config, &project);
        let estimated_cost = estimate_task_cost(
            &entry.description,
            &config.default_backend,
            config.backends.claude.as_ref().map(|c| c.model.as_str()).unwrap_or("claude-sonnet-4-20250514"),
        );

        let desc_short = if entry.description.len() > 40 {
            format!("{}...", &entry.description[..40])
        } else {
            entry.description.clone()
        };

        match orchestrator
            .submit_task_with_context(
                task,
                project_config,
                estimated_cost,
                Some(project.path.clone()),
                Vec::new(),
                crate::adapters::traits::ExecutionConstraints::default(),
            )
            .await
        {
            Ok(result) => {
                let cost = result
                    .usage
                    .as_ref()
                    .map(|u| u.cost)
                    .unwrap_or(MoneyAmount::ZERO);
                total_cost = total_cost + cost;
                println!(
                    "[{}/{}] OK   {:?} -> {} | {}",
                    i + 1,
                    total,
                    task_type,
                    result.routing_decision.selected_backend,
                    desc_short
                );
                succeeded += 1;
            }
            Err(e) => {
                println!("[{}/{}] FAIL {:?} | {} — {}", i + 1, total, task_type, desc_short, e);
                failed += 1;
            }
        }
    }

    println!("\n{}", "=".repeat(50));
    println!(
        "Batch complete: {} succeeded, {} failed, total cost: {}",
        succeeded, failed, total_cost
    );

    Ok(())
}

async fn cmd_dry_run(
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
    let estimated_cost = estimate_task_cost(
        description,
        &config.default_backend,
        config.backends.claude.as_ref().map(|c| c.model.as_str()).unwrap_or("claude-sonnet-4-20250514"),
    );

    let routing_request = crate::routing::engine::RoutingRequest {
        task_id: task.id.clone(),
        task_type: task.task_type,
        project_id: task.project_id.clone(),
        project_config,
        backend_override: task.backend_override.clone(),
        estimated_cost,
    };

    println!("Dry-run routing for {:?} task in '{}'...\n", task_type, project_name);

    match orchestrator.routing_engine.route(routing_request).await {
        Ok(decision) => {
            println!("Selected backend: {}", decision.selected_backend);
            println!("Reason:           {:?}", decision.reason);
            println!("Fallback applied: {}", decision.fallback_applied);
            println!("Budget downgrade: {}", decision.budget_downgrade_applied);

            if !decision.evaluated_backends.is_empty() {
                println!("\nEvaluated backends:");
                for eval in &decision.evaluated_backends {
                    let status = if eval.eligible {
                        "eligible"
                    } else {
                        "rejected"
                    };
                    let reason = eval.rejection_reason.as_deref().unwrap_or("-");
                    println!("  {:<15} {:<10} {}", eval.backend_id, status, reason);
                }
            }

            println!("\nEstimated cost: {}", estimated_cost);

            // Show budget state
            let year_month = Utc::now().format("%Y-%m").to_string();
            let global_limit = MoneyAmount::from_dollars(config.monthly_budget_dollars);
            if let Ok(summary) = orchestrator.budget_summary(global_limit, &year_month) {
                println!(
                    "Budget:          {} / {} ({}%)",
                    summary.global_spent,
                    summary.global_limit,
                    summary.global_spent.percentage_of(summary.global_limit)
                );
            }

            println!("\nNo task was created. This was a dry run.");
        }
        Err(e) => {
            println!("Routing failed: {}", e);
            println!("\nNo task was created. This was a dry run.");
        }
    }

    Ok(())
}

async fn cmd_queue(sub: QueueCommands, config: &GlobalConfig) -> Result<()> {
    let storage = Arc::new(open_storage(config)?);
    let orchestrator = build_orchestrator(config, Arc::clone(&storage))?;

    match sub {
        QueueCommands::List => {
            let queued = orchestrator.list_queued_tasks()?;
            if queued.is_empty() {
                println!("No tasks in queue.");
            } else {
                println!("{:<12} {:<10} {:<20} {}", "ID", "PRIORITY", "TYPE", "DESCRIPTION");
                println!("{}", "-".repeat(70));
                for task in &queued {
                    let id_short = &task.id.0.to_string()[..8];
                    let desc_short = if task.description.len() > 30 {
                        format!("{}...", &task.description[..30])
                    } else {
                        task.description.clone()
                    };
                    println!(
                        "{:<12} {:<10} {:<20} {}",
                        id_short,
                        format!("{:?}", task.priority),
                        format!("{:?}", task.task_type),
                        desc_short
                    );
                }
                println!("\n{} task(s) in queue", queued.len());
            }
        }
        QueueCommands::Run { project } => {
            let proj = storage
                .get_project_by_name(&project)?
                .ok_or_else(|| anyhow::anyhow!("project '{}' not found", project))?;
            let project_config = build_project_routing_config(config, &proj);
            let estimated_cost = MoneyAmount::from_cents(100); // default estimate
            match orchestrator.run_next_queued(project_config, estimated_cost).await? {
                Some(result) => {
                    println!("Dequeued and ran task: {}", result.task.id.0);
                    println!(
                        "Routed to: {} (reason: {:?})",
                        result.routing_decision.selected_backend, result.routing_decision.reason
                    );
                    if let Some(output) = result.execution_output {
                        println!("\n--- Output ---\n{}", output);
                    }
                }
                None => {
                    println!("Queue is empty. Nothing to run.");
                }
            }
        }
        QueueCommands::Count => {
            let count = orchestrator.count_queued_tasks()?;
            println!("{} task(s) in queue", count);
        }
    }

    Ok(())
}

fn cmd_webhooks(sub: WebhookCommands, config: &GlobalConfig) -> Result<()> {
    match sub {
        WebhookCommands::List => {
            let webhooks = config.webhooks.as_ref().map(|v| v.as_slice()).unwrap_or(&[]);
            if webhooks.is_empty() {
                println!("No webhooks configured.");
            } else {
                println!("{:<20} {:<40} {:<10} {}", "NAME", "URL", "ENABLED", "EVENTS");
                println!("{}", "-".repeat(80));
                for wh in webhooks {
                    let events = wh.events.as_ref()
                        .map(|e| e.join(", "))
                        .unwrap_or_else(|| "all".into());
                    println!(
                        "{:<20} {:<40} {:<10} {}",
                        wh.name, wh.url, wh.enabled, events
                    );
                }
            }
        }
        WebhookCommands::Test { name } => {
            let storage = Arc::new(open_storage(config)?);
            let orchestrator = build_orchestrator(config, Arc::clone(&storage))?;
            let webhooks = config.webhooks.as_ref().map(|v| v.as_slice()).unwrap_or(&[]);
            let wh = webhooks.iter().find(|w| w.name == name)
                .ok_or_else(|| anyhow::anyhow!("webhook '{}' not found", name))?;

            let test_event = Event::new(
                EventType::WebhookDispatched,
                serde_json::json!({"test": true, "webhook": wh.name}),
            );
            orchestrator.dispatch_webhooks(&test_event);
            println!("Test event dispatched to webhook '{}'", name);
        }
        WebhookCommands::Deliveries { limit } => {
            let storage = open_storage(config)?;
            let deliveries = storage.list_webhook_deliveries(limit)?;
            if deliveries.is_empty() {
                println!("No webhook deliveries recorded.");
            } else {
                println!("{:<20} {:<20} {:<20} {:<8} {}", "TIMESTAMP", "WEBHOOK", "EVENT", "OK", "URL");
                println!("{}", "-".repeat(80));
                for d in &deliveries {
                    let ts = d.delivered_at.format("%Y-%m-%d %H:%M:%S");
                    println!(
                        "{:<20} {:<20} {:<20} {:<8} {}",
                        ts,
                        d.webhook_name,
                        format!("{:?}", d.event_type),
                        if d.success { "yes" } else { "no" },
                        d.url
                    );
                }
                println!("\n{} delivery(ies)", deliveries.len());
            }
        }
    }
    Ok(())
}

fn cmd_templates(sub: TemplateCommands, config: &GlobalConfig) -> Result<()> {
    let templates = config.templates.as_ref().map(|v| v.as_slice()).unwrap_or(&[]);

    match sub {
        TemplateCommands::List => {
            if templates.is_empty() {
                println!("No templates configured.");
            } else {
                println!("{:<20} {:<25} {}", "NAME", "TASK TYPE", "DESCRIPTION");
                println!("{}", "-".repeat(70));
                for tmpl in templates {
                    let desc = tmpl.description.as_deref().unwrap_or("-");
                    let desc_short = if desc.len() > 30 {
                        format!("{}...", &desc[..30])
                    } else {
                        desc.to_string()
                    };
                    println!("{:<20} {:<25} {}", tmpl.name, tmpl.task_type, desc_short);
                }
            }
        }
        TemplateCommands::Show { name } => {
            let tmpl = templates.iter().find(|t| t.name == name)
                .ok_or_else(|| anyhow::anyhow!("template '{}' not found", name))?;
            println!("Template: {}", tmpl.name);
            println!("Task type: {}", tmpl.task_type);
            if let Some(ref desc) = tmpl.description {
                println!("Description: {}", desc);
            }
            if let Some(ref backend) = tmpl.backend {
                println!("Backend: {}", backend);
            }
            if let Some(ref priority) = tmpl.priority {
                println!("Priority: {}", priority);
            }
            if let Some(tokens) = tmpl.max_tokens {
                println!("Max tokens: {}", tokens);
            }
            if let Some(timeout) = tmpl.timeout {
                println!("Timeout: {}s", timeout);
            }
            if let Some(cost) = tmpl.max_cost {
                println!("Max cost: {} cents", cost);
            }
            let errors = tmpl.validate();
            if !errors.is_empty() {
                println!("\nValidation errors:");
                for err in &errors {
                    println!("  - {}", err);
                }
            }
        }
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
    let mut orchestrator = Orchestrator::new(registry, routing_engine, governor, storage);
    if let Some(ref retry_cfg) = config.retry_policy {
        orchestrator.retry_policy = crate::orchestrator::service::RetryPolicy {
            max_retries: retry_cfg.max_retries,
            retry_delay: std::time::Duration::from_millis(retry_cfg.retry_delay_ms),
            backoff_multiplier: retry_cfg.backoff_multiplier,
            max_delay: std::time::Duration::from_millis(retry_cfg.max_delay_ms),
            jitter_fraction: retry_cfg.jitter_fraction,
        };
    }
    if let Some(ref webhooks) = config.webhooks {
        orchestrator.webhooks = webhooks.clone();
    }
    if let Some(ref rate_limits) = config.rate_limits {
        orchestrator.rate_limits = rate_limits.clone();
    }
    if let Some(ref concurrency) = config.concurrency {
        orchestrator.concurrency = Some(concurrency.clone());
    }
    if let Some(ref cb) = config.circuit_breaker {
        orchestrator.circuit_breaker = cb.clone();
    }
    Ok(orchestrator)
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
