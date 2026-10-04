use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;
use wallpp::cache::CacheManager;
use wallpp::config::{AppConfig, SourceConfig};
use wallpp::engine::WallpaperEngine;
use wallpp::prefetch;
use wallpp::provider::{DiscoveredProvider, ProviderManager};
use wallpp::state::{State, WallpaperMetadata};

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub enum Action {
    Next,
    Previous,
    SelectHistory(usize),
    SelectPlanned(usize),
    RefillQueue,
    UpdateConfig(AppConfig),
    RefillQueueAndRefresh,
    ValidateSourceConfig { source: SourceConfig },
    FetchSourcePreviews { source: SourceConfig, count: usize },
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct SourcePreviewItem {
    pub id: String,
    pub title: Option<String>,
    pub author: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub cache_path: PathBuf,
}

#[derive(Clone, Debug)]
pub enum UiUpdate {
    StateUpdated {
        state: State,
        status: String,
    },
    ProvidersDiscovered(Vec<DiscoveredProvider>),
    SourceConfigNormalized {
        normalized: Vec<wallpp::wallpp::provider::types::ConfigEntry>,
    },
    SourcePreviews {
        #[allow(dead_code)]
        provider: String,
        items: Vec<SourcePreviewItem>,
    },
    SourcePreviewError(String),
    StatusMessage(String),
    Error(String),
}

pub struct BackendHandle {
    sender: mpsc::UnboundedSender<Action>,
}

impl BackendHandle {
    pub fn send(&self, action: Action) {
        let _ = self.sender.send(action);
    }
}

pub fn start_backend(
    initial_config: AppConfig,
    initial_state: State,
    ui_sender: async_channel::Sender<UiUpdate>,
) -> BackendHandle {
    let (action_tx, mut action_rx) = mpsc::unbounded_channel::<Action>();

    tokio::spawn(async move {
        let mut app_cfg = initial_config;
        let mut state = initial_state;

        let cache_mgr = match CacheManager::new() {
            Ok(c) => Arc::new(c),
            Err(e) => {
                let _ = ui_sender
                    .send(UiUpdate::Error(format!("Failed to init cache: {}", e)))
                    .await;
                return;
            }
        };

        let _ = ui_sender
            .send(UiUpdate::StatusMessage("Discovering providers...".into()))
            .await;
        let provider_mgr = match ProviderManager::new().await {
            Ok(p) => Arc::new(p),
            Err(e) => {
                let _ = ui_sender
                    .send(UiUpdate::Error(format!("Failed to init providers: {}", e)))
                    .await;
                return;
            }
        };

        let prov_list: Vec<DiscoveredProvider> = provider_mgr.providers.values().cloned().collect();
        let _ = ui_sender
            .send(UiUpdate::ProvidersDiscovered(prov_list))
            .await;

        if state.prefetch_queue.len() < 5 {
            let _ = ui_sender
                .send(UiUpdate::StatusMessage(
                    "Pre-fetching planned wallpapers...".into(),
                ))
                .await;
            let mut refill_cfg = app_cfg.clone();
            if refill_cfg.manager.prefetch_count < 5 {
                refill_cfg.manager.prefetch_count = 5;
            }
            let _ =
                prefetch::refill_prefetch_queue(&mut state, &refill_cfg, &provider_mgr, &cache_mgr)
                    .await;
            let _ = state.save();
        }

        let _ = ui_sender
            .send(UiUpdate::StateUpdated {
                state: state.clone(),
                status: "Ready".into(),
            })
            .await;

        while let Some(action) = action_rx.recv().await {
            match action {
                Action::Next => {
                    let _ = ui_sender
                        .send(UiUpdate::StatusMessage(
                            "Advancing to next wallpaper...".into(),
                        ))
                        .await;
                    let mut engine =
                        WallpaperEngine::new(&app_cfg, &mut state, &cache_mgr, &provider_mgr);
                    match engine.transition_next(None, false).await {
                        Ok(()) => {
                            let mut refill_cfg = app_cfg.clone();
                            if refill_cfg.manager.prefetch_count < 5 {
                                refill_cfg.manager.prefetch_count = 5;
                            }
                            let _ = prefetch::refill_prefetch_queue(
                                &mut state,
                                &refill_cfg,
                                &provider_mgr,
                                &cache_mgr,
                            )
                            .await;
                            let _ = state.save();
                            let _ = ui_sender
                                .send(UiUpdate::StateUpdated {
                                    state: state.clone(),
                                    status: "Next wallpaper applied".into(),
                                })
                                .await;
                        }
                        Err(e) => {
                            let _ = ui_sender
                                .send(UiUpdate::Error(format!("Failed to advance: {}", e)))
                                .await;
                        }
                    }
                }
                Action::Previous => {
                    let _ = ui_sender
                        .send(UiUpdate::StatusMessage(
                            "Restoring previous wallpaper...".into(),
                        ))
                        .await;
                    let mut engine =
                        WallpaperEngine::new(&app_cfg, &mut state, &cache_mgr, &provider_mgr);
                    match engine.transition_previous().await {
                        Ok(()) => {
                            let _ = ui_sender
                                .send(UiUpdate::StateUpdated {
                                    state: state.clone(),
                                    status: "Previous wallpaper restored".into(),
                                })
                                .await;
                        }
                        Err(e) => {
                            let _ = ui_sender
                                .send(UiUpdate::Error(format!(
                                    "Failed to restore previous: {}",
                                    e
                                )))
                                .await;
                        }
                    }
                }
                Action::SelectHistory(idx) => {
                    if let Some(meta) = state.history.get(idx).cloned() {
                        let _ = ui_sender
                            .send(UiUpdate::StatusMessage(
                                "Restoring historical wallpaper...".into(),
                            ))
                            .await;
                        state.current_history_index = idx;
                        let mut engine =
                            WallpaperEngine::new(&app_cfg, &mut state, &cache_mgr, &provider_mgr);
                        match engine.set_and_record_wallpaper(meta, false).await {
                            Ok(()) => {
                                let _ = ui_sender
                                    .send(UiUpdate::StateUpdated {
                                        state: state.clone(),
                                        status: "Wallpaper restored from history".into(),
                                    })
                                    .await;
                            }
                            Err(e) => {
                                let _ = ui_sender
                                    .send(UiUpdate::Error(format!(
                                        "Failed to set wallpaper: {}",
                                        e
                                    )))
                                    .await;
                            }
                        }
                    }
                }
                Action::SelectPlanned(idx) => {
                    if idx < state.prefetch_queue.len() {
                        let _ = ui_sender
                            .send(UiUpdate::StatusMessage(
                                "Applying planned wallpaper...".into(),
                            ))
                            .await;
                        let prefetched = state.prefetch_queue.remove(idx);
                        let meta = WallpaperMetadata::from(prefetched);
                        let mut engine =
                            WallpaperEngine::new(&app_cfg, &mut state, &cache_mgr, &provider_mgr);
                        match engine.set_and_record_wallpaper(meta, true).await {
                            Ok(()) => {
                                let mut refill_cfg = app_cfg.clone();
                                if refill_cfg.manager.prefetch_count < 5 {
                                    refill_cfg.manager.prefetch_count = 5;
                                }
                                let _ = prefetch::refill_prefetch_queue(
                                    &mut state,
                                    &refill_cfg,
                                    &provider_mgr,
                                    &cache_mgr,
                                )
                                .await;
                                let _ = state.save();
                                let _ = ui_sender
                                    .send(UiUpdate::StateUpdated {
                                        state: state.clone(),
                                        status: "Planned wallpaper applied".into(),
                                    })
                                    .await;
                            }
                            Err(e) => {
                                let _ = ui_sender
                                    .send(UiUpdate::Error(format!(
                                        "Failed to apply planned wallpaper: {}",
                                        e
                                    )))
                                    .await;
                            }
                        }
                    }
                }
                Action::RefillQueue => {
                    let _ = ui_sender
                        .send(UiUpdate::StatusMessage(
                            "Refilling prefetch queue...".into(),
                        ))
                        .await;
                    let mut refill_cfg = app_cfg.clone();
                    if refill_cfg.manager.prefetch_count < 5 {
                        refill_cfg.manager.prefetch_count = 5;
                    }
                    let _ = prefetch::refill_prefetch_queue(
                        &mut state,
                        &refill_cfg,
                        &provider_mgr,
                        &cache_mgr,
                    )
                    .await;
                    let _ = state.save();
                    let _ = ui_sender
                        .send(UiUpdate::StateUpdated {
                            state: state.clone(),
                            status: "Prefetch queue updated".into(),
                        })
                        .await;
                }
                Action::UpdateConfig(new_cfg) => {
                    let _ = new_cfg.save();
                    app_cfg = new_cfg;
                }
                Action::RefillQueueAndRefresh => {
                    let _ = ui_sender
                        .send(UiUpdate::StatusMessage(
                            "Updating planned wallpapers...".into(),
                        ))
                        .await;
                    state.prefetch_queue.clear();
                    let mut refill_cfg = app_cfg.clone();
                    if refill_cfg.manager.prefetch_count < 5 {
                        refill_cfg.manager.prefetch_count = 5;
                    }
                    let _ = prefetch::refill_prefetch_queue(
                        &mut state,
                        &refill_cfg,
                        &provider_mgr,
                        &cache_mgr,
                    )
                    .await;
                    let _ = state.save();
                    let _ = ui_sender
                        .send(UiUpdate::StateUpdated {
                            state: state.clone(),
                            status: "Planned wallpapers updated".into(),
                        })
                        .await;
                }
                Action::ValidateSourceConfig { source } => {
                    let provider_name = source.provider.clone();
                    if let Some(prov) = provider_mgr.providers.get(&provider_name) {
                        let wit_cfg =
                            source.to_wit_config(&prov.info.options, &prov.info.default_config);
                        if let Ok(valid_cfg) =
                            provider_mgr.validate_config(&provider_name, wit_cfg).await
                        {
                            let _ = ui_sender
                                .send(UiUpdate::SourceConfigNormalized {
                                    normalized: valid_cfg,
                                })
                                .await;
                        }
                    }
                }
                Action::FetchSourcePreviews { source, count } => {
                    let provider_name = source.provider.clone();
                    let _ = ui_sender
                        .send(UiUpdate::StatusMessage(format!(
                            "Fetching {} previews for {}...",
                            count, provider_name
                        )))
                        .await;
                    if let Some(prov) = provider_mgr.providers.get(&provider_name) {
                        let wit_cfg =
                            source.to_wit_config(&prov.info.options, &prov.info.default_config);
                        let valid_cfg =
                            match provider_mgr.validate_config(&provider_name, wit_cfg).await {
                                Ok(c) => {
                                    let _ = ui_sender
                                        .send(UiUpdate::SourceConfigNormalized {
                                            normalized: c.clone(),
                                        })
                                        .await;
                                    c
                                }
                                Err(e) => {
                                    let _ = ui_sender
                                        .send(UiUpdate::SourcePreviewError(format!(
                                            "Config error: {}",
                                            e
                                        )))
                                        .await;
                                    continue;
                                }
                            };
                        let filter = wallpp::monitor::compute_filter_criteria(
                            app_cfg.manager.min_display_percentage,
                        );
                        match provider_mgr
                            .query_list(
                                &provider_name,
                                valid_cfg.clone(),
                                count as u32,
                                None,
                                filter,
                            )
                            .await
                        {
                            Ok(page) => {
                                let mut items = Vec::new();
                                for item in page.items.into_iter().take(count) {
                                    let cache_path = if let Some(p) =
                                        cache_mgr.get_cached_image(&item.id)
                                    {
                                        Some(p)
                                    } else if let Ok(img) = provider_mgr
                                        .query_download(&provider_name, valid_cfg.clone(), &item.id)
                                        .await
                                    {
                                        cache_mgr
                                            .store_image(&item.id, &img.content_type, &img.data)
                                            .ok()
                                    } else {
                                        None
                                    };
                                    if let Some(path) = cache_path {
                                        items.push(SourcePreviewItem {
                                            id: item.id,
                                            title: item.title,
                                            author: item.author,
                                            width: item.width,
                                            height: item.height,
                                            cache_path: path,
                                        });
                                    }
                                }
                                if items.is_empty() {
                                    let _ = ui_sender
                                        .send(UiUpdate::SourcePreviewError(
                                            "No wallpapers found matching criteria".into(),
                                        ))
                                        .await;
                                } else {
                                    let _ = ui_sender
                                        .send(UiUpdate::SourcePreviews {
                                            provider: provider_name,
                                            items,
                                        })
                                        .await;
                                    let _ = ui_sender
                                        .send(UiUpdate::StatusMessage(
                                            "Source previews loaded".into(),
                                        ))
                                        .await;
                                }
                            }
                            Err(e) => {
                                let _ = ui_sender
                                    .send(UiUpdate::SourcePreviewError(format!(
                                        "Query failed: {}",
                                        e
                                    )))
                                    .await;
                            }
                        }
                    } else {
                        let _ = ui_sender
                            .send(UiUpdate::SourcePreviewError(format!(
                                "Provider '{}' not found",
                                provider_name
                            )))
                            .await;
                    }
                }
            }
        }
    });

    BackendHandle { sender: action_tx }
}
