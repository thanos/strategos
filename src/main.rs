use anyhow::Result;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    // Parse CLI to get config path before initializing tracing
    let (config, cli) = strategos::cli::parse_config()?;

    // Initialize tracing: RUST_LOG env takes precedence, then config, then "info"
    let default_level = config.log_level.as_deref().unwrap_or("info");
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new(default_level)),
        )
        .with_target(false)
        .init();

    // Validate config and warn on errors
    let validation_errors = config.validate();
    for err in &validation_errors {
        tracing::warn!("config: {}", err);
    }

    strategos::cli::run_with(cli, config).await
}
