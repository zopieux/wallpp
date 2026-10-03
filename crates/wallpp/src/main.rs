use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

mod cache;
mod config;
mod provider;

use cache::CacheManager;
use config::{AppConfig, SourceConfig};
use provider::ProviderManager;

wasmtime::component::bindgen!({
    path: "../../wit",
    world: "wallpaper-provider",
    exports: { default: async },
});

#[derive(Parser)]
#[command(name = "wallpp", about = "wallpp, a minimal wallpaper manager")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Pick the next wallpaper and download it (default)
    Next {
        #[arg(short, long)]
        source: Option<String>,
    },
    /// List available wallpapers from sources without downloading
    List {
        #[arg(short, long)]
        source: Option<String>,
        #[arg(short, long, default_value = "10")]
        limit: u32,
    },
    /// Show discovered providers and their configuration options
    Providers,
    /// List provider search directories and their existence status
    SearchPaths,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let cache_mgr = CacheManager::new().context("Failed to initialize cache manager")?;
    let provider_mgr = ProviderManager::new().await.context("Failed to initialize provider manager")?;

    match cli.command.unwrap_or(Commands::Next { source: None }) {
        Commands::SearchPaths => {
            println!("Provider search paths:");
            for dir in ProviderManager::search_dirs() {
                let status = if dir.exists() { "exists" } else { "not found" };
                println!("  - {:?} ({})", dir, status);
            }
        }
        Commands::Providers => {
            if provider_mgr.providers.is_empty() {
                println!("No providers found. Run 'wallpp search-paths' or set WALLPP_PROVIDERS_DIR.");
            } else {
                for (name, p) in &provider_mgr.providers {
                    println!("Provider: {} (v{})", name, p.info.version);
                    println!("  File: {:?}", p.path);
                    println!("  Allowed hosts: {:?}", p.info.allowed_hosts);
                    println!("  Options:");
                    for opt in &p.info.options {
                        let req = if opt.required { "required" } else { "optional" };
                        let mult = if opt.multiple { "[]" } else { "" };
                        println!("    - {}{}: {:?} ({})", opt.key, mult, opt.ty, req);
                        if let Some(ref desc) = opt.description {
                            println!("        {}", desc);
                        }
                    }
                    println!();
                }
            }
        }
        Commands::List { source, limit } => {
            let app_cfg = AppConfig::load_or_default()?;
            let sources = filter_sources(&app_cfg.source, source.as_deref());

            if sources.is_empty() {
                eprintln!("No matching sources found in configuration.");
                return Ok(());
            }

            for src in sources {
                let name = src.name.as_deref().unwrap_or(&src.provider);
                println!("=== Source: {} (provider: {}) ===", name, src.provider);

                let discovered = match provider_mgr.providers.get(&src.provider) {
                    Some(p) => p,
                    None => {
                        eprintln!("  Provider '{}' is not installed or discovered.", src.provider);
                        continue;
                    }
                };

                let wit_cfg = config::toml_to_wit_config(&src.options, &discovered.info.options);
                let filter = wallpp::provider::types::FilterCriteria {
                    min_width: None,
                    min_height: None,
                    orientation: None,
                };

                match provider_mgr.query_list(&src.provider, wit_cfg, limit, None, filter).await {
                    Ok(page) => {
                        println!("Found {} wallpapers:", page.items.len());
                        for (i, w) in page.items.iter().enumerate() {
                            let dims = match (w.width, w.height) {
                                (Some(w), Some(h)) => format!(" [{}x{}]", w, h),
                                _ => String::new(),
                            };
                            let author = w.author.as_deref().unwrap_or("unknown");
                            let title = w.title.as_deref().unwrap_or("(no title)");
                            println!("  {}. {}{} by {}", i + 1, title, dims, author);
                            if let Some(ref s) = w.source_url {
                                println!("     Source: {}", s);
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("  Error querying provider: {}", e);
                    }
                }
                println!();
            }
        }
        Commands::Next { source } => {
            let app_cfg = AppConfig::load_or_default()?;
            let sources = filter_sources(&app_cfg.source, source.as_deref());

            if sources.is_empty() {
                anyhow::bail!("No matching sources found in configuration.");
            }

            // Pick first source (or round-robin / random)
            let src = &sources[0];
            let src_name = src.name.as_deref().unwrap_or(&src.provider);

            let discovered = provider_mgr
                .providers
                .get(&src.provider)
                .with_context(|| format!("Provider '{}' not found", src.provider))?;

            let wit_cfg = config::toml_to_wit_config(&src.options, &discovered.info.options);
            let filter = wallpp::provider::types::FilterCriteria {
                min_width: None,
                min_height: None,
                orientation: None,
            };

            // Query wallpapers
            let page = provider_mgr
                .query_list(&src.provider, wit_cfg.clone(), 30, None, filter)
                .await?;

            if page.items.is_empty() {
                anyhow::bail!("Source '{}' returned 0 wallpapers", src_name);
            }

            let selected = &page.items[0];
            let id = &selected.id;

            // Check image cache or download
            let cached_path = match cache_mgr.get_cached_image(id) {
                Some(p) => {
                    println!("[Cache hit] Image already cached at {:?}", p);
                    p
                }
                None => {
                    println!("[Downloading] Fetching image from provider...");
                    let img = provider_mgr.query_download(&src.provider, wit_cfg, id).await?;
                    let p = cache_mgr.store_image(id, &img.content_type, &img.data)?;
                    println!("[Downloaded] Stored {} bytes at {:?}", img.data.len(), p);
                    p
                }
            };

            println!("\n=== Selected Wallpaper ===");
            if let Some(ref t) = selected.title {
                println!("Title:       {}", t);
            }
            if let Some(ref a) = selected.author {
                println!("Author:      {}", a);
            }
            if let (Some(w), Some(h)) = (selected.width, selected.height) {
                println!("Dimensions:  {}x{}", w, h);
            }
            if let Some(ref s) = selected.source_url {
                println!("Source URL:  {}", s);
            }
            println!("Cache path:  {}", cached_path.display());
            println!("(Wallpaper rotation applied - printing only as requested)");
        }
    }

    Ok(())
}

fn filter_sources<'a>(sources: &'a [SourceConfig], filter: Option<&str>) -> Vec<&'a SourceConfig> {
    match filter {
        Some(name) => sources
            .iter()
            .filter(|s| s.name.as_deref() == Some(name) || s.provider == name)
            .collect(),
        None => sources.iter().collect(),
    }
}
