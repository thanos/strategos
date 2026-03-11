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
use crate::models::project::Project;
use crate::models::task::Task;
use crate::models::{BackendId, MoneyAmount, PrivacyLevel, TaskType};
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
enum Commands {
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

    /// Show current configuration
    Config,
}

#[derive(Subcommand)]
enum ProjectCommands {
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

pub async fn run() -> Result<()> {
    let cli = Cli::parse();

    let config_path = cli.config.unwrap_or_else(GlobalConfig::default_path);
    let config = if config_path.exists() {
        GlobalConfig::load(&config_path)?
    } else {
        GlobalConfig::sample()
    };

    match cli.command {
        Commands::Init => cmd_init(&config_path, &config),
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
        Commands::Config => cmd_config(&config, &config_path),
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

    let (registry, governor) = build_runtime(config)?;
    let routing_engine = RoutingEngine::new(
        RoutingPolicy::default(),
        Arc::clone(&registry),
        Arc::clone(&governor),
    );

    let orchestrator = Orchestrator::new(registry, routing_engine, governor, storage);

    let mut task = Task::new(project.id.clone(), task_type, description);
    if let Some(ref b) = backend_override {
        task.backend_override = Some(BackendId::new(b));
    }

    let project_config = ProjectRoutingConfig {
        default_backend: project.default_backend.clone(),
        fallback_chain: project.fallback_chain.clone(),
        privacy: project.privacy,
        task_overrides: HashMap::new(),
    };

    println!("Submitting {:?} task to project '{}'...", task_type, project_name);

    let result = orchestrator
        .submit_task(task, project_config, MoneyAmount::from_cents(100))
        .await?;

    println!(
        "Routed to: {} (reason: {:?})",
        result.routing_decision.selected_backend, result.routing_decision.reason
    );

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
    let (registry, _) = build_runtime(config)?;

    let year_month = Utc::now().format("%Y-%m").to_string();
    let global_limit = MoneyAmount::from_dollars(config.monthly_budget_dollars);

    let routing_engine = RoutingEngine::new(
        RoutingPolicy::default(),
        Arc::clone(&registry),
        Arc::new(BudgetGovernor::new(
            BudgetConfig::default(),
            Arc::new(InMemoryUsageStore::new()),
        )),
    );

    let orchestrator = Orchestrator::new(
        registry,
        routing_engine,
        Arc::new(BudgetGovernor::new(
            BudgetConfig::default(),
            Arc::new(InMemoryUsageStore::new()),
        )),
        storage,
    );

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

fn cmd_config(config: &GlobalConfig, config_path: &PathBuf) -> Result<()> {
    println!("Config file: {}", config_path.display());
    println!("Storage: {}", config.storage_path().display());
    println!("Default backend: {}", config.default_backend);
    println!("Budget mode: {:?}", config.budget_mode);
    println!("Monthly budget: ${:.2}", config.monthly_budget_dollars);
    println!("\nBackends:");
    if let Some(ref claude) = config.backends.claude {
        println!("  claude: model={}, key_env={}", claude.model, claude.api_key_env);
    }
    if let Some(ref ollama) = config.backends.ollama {
        println!("  ollama: model={}, endpoint={}", ollama.model, ollama.endpoint);
    }
    if config.backends.opencode.is_some() {
        println!("  opencode: configured");
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

fn build_runtime(
    config: &GlobalConfig,
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
        project_limits: HashMap::new(),
        thresholds: vec![50, 75, 90, 100],
        downgrade_map,
    };

    let governor = BudgetGovernor::new(budget_config, usage_store);

    Ok((Arc::new(registry), Arc::new(governor)))
}
