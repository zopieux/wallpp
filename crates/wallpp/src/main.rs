use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::process::Command;
use std::sync::Arc;

use wallpp::cache::CacheManager;
use wallpp::config::AppConfig;
use wallpp::engine::WallpaperEngine;
use wallpp::provider::ProviderManager;
use wallpp::state::State;

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
    /// Print info for the currently set wallpaper (metadata, URL, path),
    /// without fetching or changing anything.
    Info,
    /// List provider search directories and their existence.
    SearchPaths,
    /// Open the GTK configuration editor (wallpp-gtk).
    Config,
}

/// Replaces this process with the `wallpp-gtk` editor binary (exec on Unix).
///
/// Looks for `wallpp-gtk` next to the current executable first, then falls
/// back to a PATH lookup. Only returns if the exec fails.
fn exec_gtk_editor() -> Result<()> {
    use std::os::unix::process::CommandExt;

    let sibling = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("wallpp-gtk")))
        .filter(|path| path.is_file());

    let mut cmd = match sibling {
        Some(path) => Command::new(path),
        None => Command::new("wallpp-gtk"),
    };

    let err = cmd.exec();
    Err(err).context(
        "Failed to execute wallpp-gtk configuration editor \
         (looked next to the wallpp executable, then on PATH)",
    )
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    if let Some(Commands::Config) = cli.command {
        return exec_gtk_editor();
    }

    let cache_mgr = CacheManager::new().context("Failed to initialize cache manager")?;
    let provider_mgr = Arc::new(
        ProviderManager::new()
            .await
            .context("Failed to initialize provider manager")?,
    );
    let app_cfg = AppConfig::load_or_default()?;
    let mut state = State::load_or_default();

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
        Some(Commands::Info) => {
            engine.show_current_info();
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
