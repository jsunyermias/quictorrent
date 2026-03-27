mod cli;
mod commands;
mod config;
mod daemon;
mod state;

use std::path::PathBuf;
use anyhow::Result;
use clap::Parser;
use tracing_subscriber::EnvFilter;

use cli::{Cli, Commands, FlowAction};
use config::Config;
use bitturbulence_protocol::Priority;

#[tokio::main]
async fn main() -> Result<()> {
    // Inicializar logging (RUST_LOG=info por defecto)
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("warn"))
        )
        .init();

    let cli = Cli::parse();

    // Cargar configuración
    let config_path = cli.config.unwrap_or_else(|| {
        dirs_next::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".submarine")
            .join("config.json")
    });

    let config = Config::load(&config_path)?;
    let state_path = config.state_file.clone();

    match cli.command {
        Commands::Status => {
            commands::cmd_status(&state_path)?;
        }

        Commands::Serve => {
            commands::cmd_serve(&config, &state_path).await?;
        }

        Commands::Flow { action } => match action {
            FlowAction::Add { path, save, priority } => {
                let prio = Priority::from_u8(priority)
                    .unwrap_or(Priority::Normal);
                commands::cmd_add(&path, save, prio, &state_path, &config).await?;
            }
            FlowAction::Start { id } => {
                commands::cmd_start(&id, &state_path)?;
            }
            FlowAction::Pause { id } => {
                commands::cmd_pause(&id, &state_path)?;
            }
            FlowAction::Stop { id } => {
                commands::cmd_stop(&id, &state_path)?;
            }
            FlowAction::Peers { id } => {
                commands::cmd_peers(&id, &state_path)?;
            }
            FlowAction::Create { path, name, trackers, comment, priority, output } => {
                let prio = Priority::from_u8(priority).unwrap_or(Priority::Normal);
                commands::cmd_create(&path, name, trackers, comment, prio, output)?;
            }
        },
    }

    Ok(())
}
