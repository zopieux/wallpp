//! Tokio backend: owns config/state/providers and does all network,
//! wasm, cache, and file work off the GTK thread. The UI only sends
//! `Action`s and renders `UiUpdate`s.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;
use wallpp::cache::CacheManager;
use wallpp::config::{AppConfig, SourceConfig};
use wallpp::engine::WallpaperEngine;
use wallpp::prefetch;
use wallpp::provider::{DiscoveredProvider, ProviderManager};
use wallpp::state::{State, WallpaperMetadata};
use wallpp::wallpp::provider::types::ConfigEntry;

#[derive(Clone, Debug)]
pub enum Action {
    ApplyPlanned(usize),
    ApplyHistory(usize),
    UpdateConfig(AppConfig),
    /// Clear the planned queue and refill it (settings changed).
    RefreshQueue,
    Validate(SourceConfig),
    Previews(SourceConfig, usize),
}

#[derive(Clone, Debug)]
pub struct PreviewItem {
    pub title: Option<String>,
    pub author: Option<String>,
    /// Reduced-resolution copy used by the source preview grid.
    pub thumbnail_path: PathBuf,
}

#[derive(Clone, Debug)]
pub enum UiUpdate {
    State(State),
    /// A planned-queue refill started/finished (show/hide a spinner).
    Refreshing(bool),
    Providers(Vec<DiscoveredProvider>),
    Normalized(Vec<ConfigEntry>),
    Previews(Result<Vec<PreviewItem>, String>),
}

#[derive(Clone)]
pub struct BackendHandle {
    sender: mpsc::UnboundedSender<Action>,
}

impl BackendHandle {
    pub fn send(&self, action: Action) {
        let _ = self.sender.send(action);
    }
}

type UiSender = async_channel::Sender<UiUpdate>;

struct Backend {
    cfg: AppConfig,
    state: State,
    cache: Arc<CacheManager>,
    providers: Arc<ProviderManager>,
    ui: UiSender,
}

impl Backend {
    fn publish(&self) {
        let _ = self.ui.try_send(UiUpdate::State(self.state.clone()));
    }

    fn queue_cfg(&self) -> AppConfig {
        let mut cfg = self.cfg.clone();
        cfg.manager.prefetch_count = cfg.manager.prefetch_count.max(5);
        cfg
    }

    /// Generate any missing grid thumbnails for wallpapers already in
    /// state (e.g. cached before thumbnail support existed).
    async fn ensure_state_thumbnails(&self) {
        let mut seen = HashSet::new();
        let paths: Vec<PathBuf> = self
            .state
            .history
            .iter()
            .map(|item| &item.cache_path)
            .chain(
                self.state
                    .prefetch_queue
                    .iter()
                    .map(|item| &item.cache_path),
            )
            .filter(|path| seen.insert((*path).clone()))
            .cloned()
            .collect();
        for path in paths {
            if self.cache.thumbnail_path(&path).is_file() {
                continue;
            }
            let cache = Arc::clone(&self.cache);
            let _ = tokio::task::spawn_blocking(move || cache.ensure_thumbnail(&path)).await;
        }
    }

    /// Refill the planned queue, publishing state after each new item.
    async fn refill(&mut self) {
        let _ = self.ui.try_send(UiUpdate::Refreshing(true));
        self.ensure_state_thumbnails().await;
        let cfg = self.queue_cfg();
        let ui = self.ui.clone();
        let mut progress = move |st: &State| {
            let _ = ui.try_send(UiUpdate::State(st.clone()));
        };
        let _ = prefetch::refill_prefetch_queue(
            &mut self.state,
            &cfg,
            &self.providers,
            &self.cache,
            &mut progress,
        )
        .await;
        let _ = self.state.save();
        self.publish();
        let _ = self.ui.try_send(UiUpdate::Refreshing(false));
    }

    async fn apply(&mut self, meta: WallpaperMetadata, is_new: bool) {
        let mut engine =
            WallpaperEngine::new(&self.cfg, &mut self.state, &self.cache, &self.providers);
        if engine.set_and_record_wallpaper(meta, is_new).await.is_ok() && is_new {
            self.refill().await;
        } else {
            let _ = self.state.save();
            self.publish();
        }
    }

    /// Convert a source to its provider-validated WIT config, publishing
    /// the normalized form so the UI can show canonical values.
    async fn validated_config(
        &self,
        source: &SourceConfig,
    ) -> Result<wallpp::wallpp::provider::types::Config, String> {
        let prov = self
            .providers
            .providers
            .get(&source.provider)
            .ok_or_else(|| format!("Provider '{}' not found", source.provider))?;
        let wit_cfg = source.to_wit_config(&prov.info.options, &prov.info.default_config);
        let valid = self
            .providers
            .validate_config(&source.provider, wit_cfg)
            .await
            .map_err(|e| format!("Config error: {e}"))?;
        let _ = self.ui.try_send(UiUpdate::Normalized(valid.clone()));
        Ok(valid)
    }

    async fn previews(&self, source: SourceConfig, count: usize) {
        let send = |u: UiUpdate| {
            let _ = self.ui.try_send(u);
        };
        let wit_cfg = match self.validated_config(&source).await {
            Ok(c) => c,
            Err(e) => {
                send(UiUpdate::Previews(Err(e)));
                return;
            }
        };
        let filter =
            wallpp::monitor::compute_filter_criteria(self.cfg.manager.min_display_percentage);
        let page = match self
            .providers
            .query_list(
                &source.provider,
                wit_cfg.clone(),
                count as u32,
                None,
                filter,
            )
            .await
        {
            Ok(p) => p,
            Err(e) => {
                send(UiUpdate::Previews(Err(format!("Query failed: {e}"))));
                return;
            }
        };
        let mut items = Vec::new();
        for item in page.items.into_iter().take(count) {
            if let Ok(cached) = prefetch::cached_image(
                &self.providers,
                &self.cache,
                &source.provider,
                &wit_cfg,
                &item.id,
            )
            .await
            {
                items.push(PreviewItem {
                    title: item.title,
                    author: item.author,
                    thumbnail_path: cached.thumbnail_path,
                });
            }
        }
        if items.is_empty() {
            send(UiUpdate::Previews(Err(
                "No wallpapers found matching criteria".into(),
            )));
        } else {
            send(UiUpdate::Previews(Ok(items)));
        }
    }
}

pub fn start_backend(cfg: AppConfig, state: State, ui: UiSender) -> BackendHandle {
    let (sender, mut rx) = mpsc::unbounded_channel::<Action>();
    let handle = BackendHandle { sender };

    tokio::spawn(async move {
        let cache = match CacheManager::new() {
            Ok(c) => Arc::new(c),
            Err(e) => {
                eprintln!("wallpp-gtk: failed to init cache: {e}");
                return;
            }
        };
        let providers = match ProviderManager::new().await {
            Ok(p) => Arc::new(p),
            Err(e) => {
                eprintln!("wallpp-gtk: failed to init providers: {e}");
                return;
            }
        };
        let _ = ui.try_send(UiUpdate::Providers(
            providers.providers.values().cloned().collect(),
        ));

        let mut backend = Backend {
            cfg,
            state,
            cache,
            providers,
            ui,
        };
        if backend.state.prefetch_queue.len() < 5 {
            backend.refill().await;
        } else {
            backend.ensure_state_thumbnails().await;
            backend.publish();
        }

        while let Some(action) = rx.recv().await {
            match action {
                Action::ApplyPlanned(idx) => {
                    if idx < backend.state.prefetch_queue.len() {
                        let meta = backend.state.prefetch_queue.remove(idx).into();
                        backend.apply(meta, true).await;
                    }
                }
                Action::ApplyHistory(idx) => {
                    if let Some(meta) = backend.state.history.get(idx).cloned() {
                        backend.state.current_history_index = idx;
                        backend.apply(meta, false).await;
                    }
                }
                Action::UpdateConfig(new_cfg) => {
                    let _ = new_cfg.save();
                    backend.cfg = new_cfg;
                }
                Action::RefreshQueue => {
                    backend.state.prefetch_queue.clear();
                    backend.refill().await;
                }
                Action::Validate(source) => {
                    let _ = backend.validated_config(&source).await;
                }
                Action::Previews(source, count) => backend.previews(source, count).await,
            }
        }
    });

    handle
}
