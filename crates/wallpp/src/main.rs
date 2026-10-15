use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::collections::HashMap;
use std::path::PathBuf;

mod cache;
mod config;
mod prefetch;
mod provider;
mod state;
mod wallpaper;

use cache::CacheManager;
use config::{AppConfig, RefreshStrategy, SourceStrategy};
use provider::ProviderManager;
use state::{current_utc_timestamp, State, WallpaperMetadata};

wasmtime::component::bindgen!({
    path: "../../wit",
    world: "wallpaper-provider",
    exports: { default: async },
});

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
    /// Pick the next wallpaper, download it, and set it on the system.
    Next {
        #[arg(short, long)]
        source: Option<String>,
    },
    /// Switch to the previous wallpaper in history.
    Previous,
    /// Show wallpaper history.
    History {
        #[arg(short, long, default_value = "10")]
        limit: usize,
    },
    /// Preview the next wallpaper by downloading and printing info without setting.
    Preview {
        #[arg(short, long)]
        source: Option<String>,
    },
    /// List available wallpapers from sources without downloading.
    List {
        #[arg(short, long)]
        source: Option<String>,
        #[arg(short, long, default_value = "10")]
        limit: u32,
    },
    /// Show discovered providers and their configuration options.
    Providers,
    /// List provider search directories and their existence status.
    SearchPaths,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let cache_mgr = CacheManager::new().context("Failed to initialize cache manager")?;
    let mut provider_mgr = ProviderManager::new()
        .await
        .context("Failed to initialize provider manager")?;
    let app_cfg = AppConfig::load_or_default()?;
    let mut state = State::load_or_default();

    let mut engine = WallpaperEngine {
        app_cfg: &app_cfg,
        state: &mut state,
        cache_mgr: &cache_mgr,
        provider_mgr: &mut provider_mgr,
    };

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
        Some(Commands::History { limit }) => {
            if engine.state.history.is_empty() {
                println!("No wallpaper history found.");
                return Ok(());
            }

            println!("Wallpaper history (total: {}):", engine.state.history.len());
            for (idx, entry) in engine.state.history.iter().take(limit).enumerate() {
                let marker = if idx == engine.state.current_history_index {
                    "*"
                } else {
                    " "
                };
                let title = entry.title.as_deref().unwrap_or(&entry.opaque_id);
                let dims = match (entry.width, entry.height) {
                    (Some(w), Some(h)) => format!(" [{}x{}]", w, h),
                    _ => String::new(),
                };
                let src = entry.source_name.as_deref().unwrap_or(&entry.provider);
                println!(" {} {:3}. [{}] {}{}", marker, idx + 1, src, title, dims);
                if let Some(ref url) = entry.source_url {
                    println!("       Source URL: {}", url);
                }
                println!("       Cache path: {}", entry.cache_path.display());
            }
        }
        Some(Commands::List { source, limit }) => {
            let sources =
                get_active_sources(engine.app_cfg, source.as_deref(), &engine.provider_mgr.providers);

            if sources.is_empty() {
                eprintln!("No matching sources found.");
                return Ok(());
            }

            for src in sources {
                println!(
                    "=== Source: {} (provider: {}) ===",
                    src.name, src.provider.name
                );

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

                match engine
                    .provider_mgr
                    .query_list(&src.provider.name, wit_cfg, limit, None, filter)
                    .await
                {
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
        Some(Commands::Providers) => {
            if engine.provider_mgr.providers.is_empty() {
                println!(
                    "No providers found. Run 'wallpp search-paths' or set WALLPP_PROVIDERS_DIR."
                );
            } else {
                for (name, p) in &engine.provider_mgr.providers {
                    println!(
                        "Provider: {} [{}] (v{})",
                        p.info.label, name, p.info.version
                    );
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
        Some(Commands::SearchPaths) => {
            println!("Provider search paths:");
            for dir in ProviderManager::search_dirs() {
                let status = if dir.exists() { "exists" } else { "not found" };
                println!("  - {:?} ({})", dir, status);
            }
        }
    }

    Ok(())
}

/// Unified Wallpaper Engine managing states, transitions, applying wallpapers, and background maintenance.
struct WallpaperEngine<'a> {
    app_cfg: &'a AppConfig,
    state: &'a mut State,
    cache_mgr: &'a CacheManager,
    provider_mgr: &'a mut ProviderManager,
}

impl<'a> WallpaperEngine<'a> {
    /// Pretty-print metadata for a candidate wallpaper.
    fn display_wallpaper_info(&self, meta: &WallpaperMetadata, header: Option<&str>) {
        if let Some(h) = header {
            println!("{}", h);
        }
        let src = meta.source_name.as_deref().unwrap_or(&meta.provider);
        println!("Source:      {} [{}]", src, meta.provider);
        if let Some(ref t) = meta.title {
            println!("Title:       {}", t);
        }
        if let Some(ref a) = meta.author {
            println!("Author:      {}", a);
        }
        if let (Some(w), Some(h)) = (meta.width, meta.height) {
            println!("Dimensions:  {}x{}", w, h);
        }
        if let Some(ref s) = meta.source_url {
            println!("Source URL:  {}", s);
        }
        println!("Cache path:  {}", meta.cache_path.display());
    }

    /// Single point of setting the desktop wallpaper, updating state/history, and performing post-maintenance.
    async fn set_and_record_wallpaper(
        &mut self,
        meta: WallpaperMetadata,
        is_new: bool,
    ) -> Result<()> {
        println!("[Applying] Setting desktop wallpaper...");
        wallpaper::set_wallpaper(&meta.cache_path).context("Failed to set desktop wallpaper")?;

        if is_new {
            self.state
                .add_to_history(meta, self.app_cfg.manager.history_length);
        } else {
            self.state.last_changed_at = Some(current_utc_timestamp());
        }
        self.state.save()?;
        println!("[Success] Wallpaper set successfully!");

        if is_new {
            self.post_apply_maintenance().await;
        }

        Ok(())
    }

    /// Background maintenance triggered after a new wallpaper is applied:.
    /// refilling the prefetch queue and enforcing the cache quota.
    async fn post_apply_maintenance(&mut self) {
        let _ = prefetch::refill_prefetch_queue(
            self.state,
            self.app_cfg,
            self.provider_mgr,
            self.cache_mgr,
        )
        .await;

        let keep_paths = collect_keep_paths(self.state);
        let _ = self
            .cache_mgr
            .enforce_max_size(self.app_cfg.manager.cache_max_size.as_bytes(), &keep_paths);
    }

    /// Transition to the next wallpaper:.
    /// 1. History forward-step (if navigating history).
    /// 2. Prefetch queue pop.
    /// 3. Live provider query & download.
    async fn transition_next(&mut self, source_filter: Option<&str>, is_preview: bool) -> Result<()> {
        // 1. History forward step.
        if !is_preview && source_filter.is_none() && self.state.current_history_index > 0 {
            if let Some(meta) = self.state.step_next().cloned() {
                if meta.cache_path.exists() {
                    println!("[Next] Advancing forward in history...");
                    self.display_wallpaper_info(&meta, None);
                    println!(
                        "  History pos: {}/{}",
                        self.state.current_history_index + 1,
                        self.state.history.len()
                    );
                    return self.set_and_record_wallpaper(meta, false).await;
                }
            }
        }

        // 2. Prefetch queue hit.
        if !is_preview {
            if let Some(prefetched) = self.state.pop_prefetched(source_filter) {
                if prefetched.cache_path.exists() {
                    println!("[Prefetch hit] Using pre-cached wallpaper from queue");
                    let meta = WallpaperMetadata::from(prefetched);
                    self.display_wallpaper_info(&meta, None);
                    return self.set_and_record_wallpaper(meta, true).await;
                }
            }
        }

        // 3. Live provider query & download.
        let meta = self.fetch_live_wallpaper(source_filter).await?;
        self.display_wallpaper_info(&meta, Some("\n=== Selected Wallpaper ==="));

        if is_preview {
            println!("(Preview mode - wallpaper not set)");
            Ok(())
        } else {
            self.set_and_record_wallpaper(meta, true).await
        }
    }

    /// Transition to the previous wallpaper in history.
    async fn transition_previous(&mut self) -> Result<()> {
        if let Some(meta) = self.state.step_previous().cloned() {
            if meta.cache_path.exists() {
                println!("[Previous] Restoring wallpaper from history...");
                self.display_wallpaper_info(&meta, None);
                println!(
                    "  History pos: {}/{}",
                    self.state.current_history_index + 1,
                    self.state.history.len()
                );
                self.set_and_record_wallpaper(meta, false).await?;
            } else {
                eprintln!(
                    "Warning: Cached wallpaper file {:?} was removed from disk.",
                    meta.cache_path
                );
            }
        } else {
            println!(
                "Already at oldest wallpaper in history ({} wallpapers recorded).",
                self.state.history.len()
            );
        }
        Ok(())
    }

    /// Query sources, select one based on manager strategy (Random or RoundRobin),.
    /// download the image if not cached, and construct WallpaperMetadata.
    async fn fetch_live_wallpaper(&mut self, source_filter: Option<&str>) -> Result<WallpaperMetadata> {
        let sources =
            get_active_sources(self.app_cfg, source_filter, &self.provider_mgr.providers);
        if sources.is_empty() {
            anyhow::bail!("No matching sources found.");
        }

        let idx = match self.app_cfg.manager.source_strategy {
            SourceStrategy::Random => {
                let seed = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or(0);
                (seed as usize) % sources.len()
            }
            SourceStrategy::RoundRobin => {
                if let Some(last) = self.state.last_source() {
                    if let Some(pos) = sources.iter().position(|s| {
                        s.name.eq_ignore_ascii_case(last)
                            || s.provider.name.eq_ignore_ascii_case(last)
                    }) {
                        (pos + 1) % sources.len()
                    } else {
                        // Edge case: source was removed or disabled between runs.
                        // Gracefully start over from the first available source.
                        0
                    }
                } else {
                    0
                }
            }
        };

        let src = &sources[idx];

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

        let page = self
            .provider_mgr
            .query_list(&src.provider.name, wit_cfg.clone(), 30, None, filter)
            .await?;

        if page.items.is_empty() {
            anyhow::bail!("Source '{}' returned 0 wallpapers", src.name);
        }

        let selected = &page.items[0];
        let id = &selected.id;

        let cached_path = match self.cache_mgr.get_cached_image(id) {
            Some(p) => {
                println!("[Cache hit] Image already cached at {:?}", p);
                p
            }
            None => {
                println!("[Downloading] Fetching image from provider...");
                let img = self
                    .provider_mgr
                    .query_download(&src.provider.name, wit_cfg, id)
                    .await?;
                let p = self.cache_mgr.store_image(id, &img.content_type, &img.data)?;
                println!("[Downloaded] Stored {} bytes at {:?}", img.data.len(), p);
                p
            }
        };

        Ok(WallpaperMetadata {
            provider: src.provider.name.clone(),
            source_name: Some(src.name.clone()),
            changed_at: current_utc_timestamp(),
            opaque_id: id.clone(),
            source_url: selected.source_url.clone(),
            cache_path: cached_path,
            title: selected.title.clone(),
            author: selected.author.clone(),
            width: selected.width,
            height: selected.height,
        })
    }

    /// Resident service implementation (empty verb / systemd Exec command).
    /// Executes the on-boot restore/refresh, and if a refresh interval is configured,.
    /// enters an async interval loop until terminated.
    async fn run_resident(&mut self) -> Result<()> {
        // --- 1. On-Boot Phase ---.
        let should_refresh_at_boot = if !self.app_cfg.manager.refresh_at_boot {
            false
        } else {
            match self.app_cfg.manager.refresh_strategy {
                RefreshStrategy::FromBoot => true,
                RefreshStrategy::FromLastChanged => {
                    if let Some(interval) = self.app_cfg.manager.refresh_interval {
                        if interval == 0 {
                            true
                        } else {
                            let now = current_utc_timestamp();
                            let last = self.state.last_changed_at.unwrap_or(0);
                            now.saturating_sub(last) >= interval
                        }
                    } else {
                        true
                    }
                }
            }
        };

        if should_refresh_at_boot {
            println!("[Boot] Refreshing wallpaper according to boot strategy...");
            self.transition_next(None, false).await?;
        } else if let Some(curr) = self.state.get_current_wallpaper().cloned() {
            if curr.cache_path.exists() {
                println!("[Boot] Restoring current wallpaper from history...");
                self.display_wallpaper_info(&curr, None);
                self.set_and_record_wallpaper(curr, false).await?;
                self.post_apply_maintenance().await;
            } else {
                println!("[Boot] Cached file missing, picking next wallpaper...");
                self.transition_next(None, false).await?;
            }
        } else {
            println!("[Boot] No history found, picking next wallpaper...");
            self.transition_next(None, false).await?;
        }

        // --- 2. Periodic Refresh Interval Phase ---.
        let interval = match self.app_cfg.manager.refresh_interval {
            Some(i) if i > 0 => i,
            _ => return Ok(()),
        };

        println!(
            "[Resident] Wallpaper refresh service active: every {}s ({:.1} min)",
            interval,
            interval as f64 / 60.0
        );
        loop {
            let wait_secs = match self.app_cfg.manager.refresh_strategy {
                RefreshStrategy::FromBoot => interval,
                RefreshStrategy::FromLastChanged => {
                    let now = current_utc_timestamp();
                    let last = self.state.last_changed_at.unwrap_or(now);
                    let elapsed = now.saturating_sub(last);
                    interval.saturating_sub(elapsed).max(1)
                }
            };

            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_secs(wait_secs)) => {
                    println!("\n[Interval] Timer fired after {}s, refreshing wallpaper...", wait_secs);
                    // Reload state from disk in case an external CLI call updated it.
                    *self.state = State::load_or_default();
                    if let Err(e) = self.transition_next(None, false).await {
                        eprintln!("[Interval Error] Failed to refresh wallpaper: {:#}", e);
                    }
                }
                _ = wait_for_shutdown() => {
                    println!("\n[Resident] Received shutdown signal. Exiting gracefully.");
                    break;
                }
            }
        }

        Ok(())
    }
}

async fn wait_for_shutdown() {
    #[cfg(unix)]
    {
        let mut sigterm = match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(s) => s,
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
                return;
            }
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {},
            _ = sigterm.recv() => {},
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

fn collect_keep_paths(state: &State) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for h in state.history.iter().take(20) {
        paths.push(h.cache_path.clone());
    }
    for p in &state.prefetch_queue {
        paths.push(p.cache_path.clone());
    }
    paths
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
            if src
                .name
                .as_deref()
                .is_some_and(|n| n.eq_ignore_ascii_case(filter_name))
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
