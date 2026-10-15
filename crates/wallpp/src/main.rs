use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::collections::HashMap;

mod cache;
mod config;
mod provider;

use cache::CacheManager;
use config::AppConfig;
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
                    println!("Provider: {} [{}] (v{})", p.info.label, name, p.info.version);
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
            let sources = get_active_sources(&app_cfg, source.as_deref(), &provider_mgr.providers);

            if sources.is_empty() {
                eprintln!("No matching sources found.");
                return Ok(());
            }

            for src in sources {
                println!("=== Source: {} (provider: {}) ===", src.name, src.provider.name);

                let wit_cfg = config::toml_to_wit_config(
                    src.options,
                    &src.provider.info.options,
                    &src.provider.info.default_config,
                );
                let filter = wallpp::provider::types::FilterCriteria {
                    min_width: None,
                    min_height: None,
                    orientation: None,
                };

                match provider_mgr.query_list(&src.provider.name, wit_cfg, limit, None, filter).await {
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
            let sources = get_active_sources(&app_cfg, source.as_deref(), &provider_mgr.providers);

            if sources.is_empty() {
                anyhow::bail!("No matching sources found.");
            }

            // Pick first source (or round-robin / random)
            let src = &sources[0];

            let wit_cfg = config::toml_to_wit_config(
                src.options,
                &src.provider.info.options,
                &src.provider.info.default_config,
            );
            let filter = wallpp::provider::types::FilterCriteria {
                min_width: None,
                min_height: None,
                orientation: None,
            };

            // Query wallpapers
            let page = provider_mgr
                .query_list(&src.provider.name, wit_cfg.clone(), 30, None, filter)
                .await?;

            if page.items.is_empty() {
                anyhow::bail!("Source '{}' returned 0 wallpapers", src.name);
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
                    let img = provider_mgr.query_download(&src.provider.name, wit_cfg, id).await?;
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

struct SourceWithInfo<'a> {
    name: String,
    provider: &'a provider::DiscoveredProvider,
    options: &'a HashMap<String, toml::Value>,
}

fn get_active_sources<'a>(
    app_cfg: &'a AppConfig,
    source_filter: Option<&str>,
    discovered: &'a HashMap<String, provider::DiscoveredProvider>,
) -> Vec<SourceWithInfo<'a>> {
    static EMPTY_MAP: std::sync::LazyLock<HashMap<String, toml::Value>> =
        std::sync::LazyLock::new(HashMap::new);

    if let Some(filter_name) = source_filter {
        for src in &app_cfg.source {
            if src.name.as_deref().is_some_and(|n| n.eq_ignore_ascii_case(filter_name))
                || src.provider.eq_ignore_ascii_case(filter_name)
            {
                if let Some(prov) = discovered.get(&src.provider) {
                    return vec![SourceWithInfo {
                        name: src.name.clone().unwrap_or_else(|| prov.info.label.clone()),
                        provider: prov,
                        options: &src.options,
                    }];
                }
            }
        }
        for (name, prov) in discovered {
            if name.eq_ignore_ascii_case(filter_name)
                || prov.info.label.eq_ignore_ascii_case(filter_name)
            {
                return vec![SourceWithInfo {
                    name: prov.info.label.clone(),
                    provider: prov,
                    options: &EMPTY_MAP,
                }];
            }
        }
        return Vec::new();
    }

    if !app_cfg.source.is_empty() {
        let mut list = Vec::new();
        for src in &app_cfg.source {
            if let Some(prov) = discovered.get(&src.provider) {
                list.push(SourceWithInfo {
                    name: src.name.clone().unwrap_or_else(|| prov.info.label.clone()),
                    provider: prov,
                    options: &src.options,
                });
            } else {
                eprintln!(
                    "Warning: Configured provider '{}' not found in search paths.",
                    src.provider
                );
            }
        }
        return list;
    }

    let mut list = Vec::new();
    let mut names: Vec<_> = discovered.keys().collect();
    names.sort();
    for name in names {
        let prov = &discovered[name];
        list.push(SourceWithInfo {
            name: prov.info.label.clone(),
            provider: prov,
            options: &EMPTY_MAP,
        });
    }
    list
}

