use anyhow::{Context, Result};
use std::path::PathBuf;

use crate::cache::CacheManager;
use crate::config::{self, AppConfig, RefreshStrategy, SourceStrategy};
use crate::monitor;
use crate::prefetch;
use crate::provider::ProviderManager;
use crate::sources::resolve_sources;
use crate::state::{current_utc_timestamp, State, WallpaperMetadata};
use crate::wallpaper;

/// Helper function to collect image paths that must be retained in cache.
pub fn collect_keep_paths(state: &State) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for h in state.history.iter().take(20) {
        paths.push(h.cache_path.clone());
    }
    for p in &state.prefetch_queue {
        paths.push(p.cache_path.clone());
    }
    paths
}

/// Unified Wallpaper Engine managing states, transitions, applying wallpapers, and background maintenance.
pub struct WallpaperEngine<'a> {
    pub app_cfg: &'a AppConfig,
    pub state: &'a mut State,
    pub cache_mgr: &'a CacheManager,
    pub provider_mgr: &'a ProviderManager,
}

impl<'a> WallpaperEngine<'a> {
    pub fn new(
        app_cfg: &'a AppConfig,
        state: &'a mut State,
        cache_mgr: &'a CacheManager,
        provider_mgr: &'a ProviderManager,
    ) -> Self {
        Self {
            app_cfg,
            state,
            cache_mgr,
            provider_mgr,
        }
    }

    /// Pretty-print metadata for a candidate wallpaper.
    pub fn display_wallpaper_info(&self, meta: &WallpaperMetadata, header: Option<&str>) {
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
    pub async fn set_and_record_wallpaper(
        &mut self,
        meta: WallpaperMetadata,
        is_new: bool,
    ) -> Result<()> {
        println!("[Applying] Setting desktop wallpaper...");
        let method = wallpaper::set_wallpaper(&meta.cache_path)
            .context("Failed to set desktop wallpaper")?;
        println!("Method:      {}", method);

        if is_new {
            self.state
                .add_to_history(meta, self.app_cfg.manager.history_length);
        } else {
            self.state.last_changed_at = Some(current_utc_timestamp());
        }
        self.state.save()?;
        println!("[Success] Wallpaper set successfully via {}!", method);

        if is_new {
            self.post_apply_maintenance().await;
        }

        Ok(())
    }

    /// Background maintenance triggered after a new wallpaper is applied:
    /// refilling the prefetch queue and enforcing the cache quota.
    pub async fn post_apply_maintenance(&mut self) {
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

    /// Transition to the next wallpaper:
    ///   1. History forward-step (if navigating history).
    ///   2. Prefetch queue pop.
    ///   3. Live provider query & download.
    pub async fn transition_next(
        &mut self,
        source_filter: Option<&str>,
        is_preview: bool,
    ) -> Result<()> {
        // History forward step.
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

        // Prefetch queue hit.
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

        // Live provider query & download.
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
    pub async fn transition_previous(&mut self) -> Result<()> {
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

    /// Query sources, select one based on manager strategy (Random or RoundRobin),
    /// download the image if not cached, and construct WallpaperMetadata.
    pub async fn fetch_live_wallpaper(
        &mut self,
        source_filter: Option<&str>,
    ) -> Result<WallpaperMetadata> {
        let sources = resolve_sources(self.app_cfg, source_filter, &self.provider_mgr.providers);
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
                        s.display_label.eq_ignore_ascii_case(last)
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
        let filter = monitor::compute_filter_criteria(self.app_cfg.manager.min_display_percentage);

        let page = self
            .provider_mgr
            .query_list(&src.provider.name, wit_cfg.clone(), 30, None, filter)
            .await?;

        if page.items.is_empty() {
            anyhow::bail!("Source '{}' returned 0 wallpapers", src.display_label);
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
                let p = self
                    .cache_mgr
                    .store_image(id, &img.content_type, &img.data)?;
                println!("[Downloaded] Stored {} bytes at {:?}", img.data.len(), p);
                p
            }
        };

        Ok(WallpaperMetadata {
            provider: src.provider.name.clone(),
            source_name: Some(src.display_label.clone()),
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
    /// Executes the on-boot restore/refresh, and if a refresh interval is configured,
    /// enters an async interval loop until terminated.
    pub async fn run_resident(&mut self) -> Result<()> {
        // On-boot phase.
        let should_refresh_at_boot = if !self.app_cfg.manager.refresh_at_boot {
            false
        } else {
            match self.app_cfg.manager.refresh_strategy {
                RefreshStrategy::FromBoot => true,
                RefreshStrategy::FromLastChanged => {
                    if let Some(interval) = self.app_cfg.manager.refresh_interval {
                        let interval_secs = interval.as_secs();
                        if interval_secs == 0 {
                            true
                        } else {
                            let now = current_utc_timestamp();
                            let last = self.state.last_changed_at.unwrap_or(0);
                            now.saturating_sub(last) >= interval_secs
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

        // Periodic refresh interval phase.
        let interval = match self.app_cfg.manager.refresh_interval {
            Some(i) if i.as_secs() > 0 => i,
            _ => return Ok(()),
        };
        let interval_secs = interval.as_secs();

        println!(
            "[Resident] Wallpaper refresh service active: every {}s ({:.1} min)",
            interval_secs,
            interval_secs as f64 / 60.0
        );
        loop {
            let wait_secs = match self.app_cfg.manager.refresh_strategy {
                RefreshStrategy::FromBoot => interval_secs,
                RefreshStrategy::FromLastChanged => {
                    let now = current_utc_timestamp();
                    let last = self.state.last_changed_at.unwrap_or(now);
                    let elapsed = now.saturating_sub(last);
                    interval_secs.saturating_sub(elapsed).max(1)
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
        let mut sigterm =
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
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
