use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::sync::Arc;

use wallpp::cache::CacheManager;
use wallpp::config::AppConfig;
use wallpp::engine::WallpaperEngine;
use wallpp::provider::ProviderManager;
use wallpp::state::State;
use wallpp::tui;

#[derive(Parser)]
#[command(
    name = "wallpp",
    about = "wallpp, a minimal wallpaper manager",
    long_about = "wallpp - a minimal wallpaper manager and resident service.\n\
    When run without subcommands, runs as the resident service handling on-boot\n\
    wallpaper setup and periodic interval refreshes."
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Pick the next wallpaper and set it on the system.
    Next {
        #[arg(short, long)]
        source: Option<String>,
    },
    /// Switch to the previous wallpaper in history.
    Previous,
    /// Preview the next wallpaper by printing info, without setting it.
    Preview {
        #[arg(short, long)]
        source: Option<String>,
    },
    /// List provider search directories and their existence.
    SearchPaths,
    /// Open the interactive TUI configuration editor.
    Config,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let cache_mgr = CacheManager::new().context("Failed to initialize cache manager")?;
    let provider_mgr = Arc::new(
        ProviderManager::new()
            .await
            .context("Failed to initialize provider manager")?,
    );
    let app_cfg = AppConfig::load_or_default()?;
    let mut state = State::load_or_default();

    if let Some(Commands::Config) = cli.command {
        return tui::run_tui(&cache_mgr, Arc::clone(&provider_mgr), &mut state).await;
    }

    let mut engine = WallpaperEngine::new(&app_cfg, &mut state, &cache_mgr, &provider_mgr);

    match cli.command {
        None => {
            engine.run_resident().await?;
        }
        Some(Commands::Next { source }) => {
            engine.transition_next(source.as_deref(), false).await?;
        }
        Some(Commands::Previous) => {
            engine.transition_previous().await?;
        }
        Some(Commands::Preview { source }) => {
            engine.transition_next(source.as_deref(), true).await?;
        }
        Some(Commands::SearchPaths) => {
            println!("Provider search paths:");
            for dir in ProviderManager::search_dirs() {
                let status = if dir.exists() { "exists" } else { "not found" };
                println!("  - {:?} ({})", dir, status);
            }
        }
        Some(Commands::Config) => unreachable!(),
    }

    Ok(())
}
