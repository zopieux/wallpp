use std::sync::Arc;
use tokio::sync::mpsc;
use wallpp::cache::CacheManager;
use wallpp::config::AppConfig;
use wallpp::engine::WallpaperEngine;
use wallpp::prefetch;
use wallpp::provider::ProviderManager;
use wallpp::state::{State, WallpaperMetadata};

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub enum Action {
    Next,
    Previous,
    SelectHistory(usize),
    SelectPlanned(usize),
    RefillQueue,
    ReloadConfig(AppConfig),
}

#[derive(Clone, Debug)]
pub enum UiUpdate {
    StateUpdated {
        state: State,
        status: String,
    },
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
                let _ = ui_sender.send(UiUpdate::Error(format!("Failed to init cache: {}", e))).await;
                return;
            }
        };

        let _ = ui_sender.send(UiUpdate::StatusMessage("Discovering providers...".into())).await;
        let provider_mgr = match ProviderManager::new().await {
            Ok(p) => Arc::new(p),
            Err(e) => {
                let _ = ui_sender.send(UiUpdate::Error(format!("Failed to init providers: {}", e))).await;
                return;
            }
        };

        if state.prefetch_queue.len() < 5 {
            let _ = ui_sender.send(UiUpdate::StatusMessage("Pre-fetching planned wallpapers...".into())).await;
            let mut refill_cfg = app_cfg.clone();
            if refill_cfg.manager.prefetch_count < 5 {
                refill_cfg.manager.prefetch_count = 5;
            }
            let _ = prefetch::refill_prefetch_queue(&mut state, &refill_cfg, &provider_mgr, &cache_mgr).await;
            let _ = state.save();
        }

        let _ = ui_sender.send(UiUpdate::StateUpdated {
            state: state.clone(),
            status: "Ready".into(),
        }).await;

        while let Some(action) = action_rx.recv().await {
            match action {
                Action::Next => {
                    let _ = ui_sender.send(UiUpdate::StatusMessage("Advancing to next wallpaper...".into())).await;
                    let mut engine = WallpaperEngine::new(&app_cfg, &mut state, &cache_mgr, &provider_mgr);
                    match engine.transition_next(None, false).await {
                        Ok(()) => {
                            let mut refill_cfg = app_cfg.clone();
                            if refill_cfg.manager.prefetch_count < 5 {
                                refill_cfg.manager.prefetch_count = 5;
                            }
                            let _ = prefetch::refill_prefetch_queue(&mut state, &refill_cfg, &provider_mgr, &cache_mgr).await;
                            let _ = state.save();
                            let _ = ui_sender.send(UiUpdate::StateUpdated {
                                state: state.clone(),
                                status: "Next wallpaper applied".into(),
                            });
                        }
                        Err(e) => {
                            let _ = ui_sender.send(UiUpdate::Error(format!("Failed to advance: {}", e))).await;
                        }
                    }
                }
                Action::Previous => {
                    let _ = ui_sender.send(UiUpdate::StatusMessage("Restoring previous wallpaper...".into())).await;
                    let mut engine = WallpaperEngine::new(&app_cfg, &mut state, &cache_mgr, &provider_mgr);
                    match engine.transition_previous().await {
                        Ok(()) => {
                            let _ = ui_sender.send(UiUpdate::StateUpdated {
                                state: state.clone(),
                                status: "Previous wallpaper restored".into(),
                            }).await;
                        }
                        Err(e) => {
                            let _ = ui_sender.send(UiUpdate::Error(format!("Failed to restore previous: {}", e))).await;
                        }
                    }
                }
                Action::SelectHistory(idx) => {
                    if let Some(meta) = state.history.get(idx).cloned() {
                        let _ = ui_sender.send(UiUpdate::StatusMessage("Restoring historical wallpaper...".into())).await;
                        state.current_history_index = idx;
                        let mut engine = WallpaperEngine::new(&app_cfg, &mut state, &cache_mgr, &provider_mgr);
                        match engine.set_and_record_wallpaper(meta, false).await {
                            Ok(()) => {
                                let _ = ui_sender.send(UiUpdate::StateUpdated {
                                    state: state.clone(),
                                    status: "Wallpaper restored from history".into(),
                                }).await;
                            }
                            Err(e) => {
                                let _ = ui_sender.send(UiUpdate::Error(format!("Failed to set wallpaper: {}", e))).await;
                            }
                        }
                    }
                }
                Action::SelectPlanned(idx) => {
                    if idx < state.prefetch_queue.len() {
                        let _ = ui_sender.send(UiUpdate::StatusMessage("Applying planned wallpaper...".into())).await;
                        let prefetched = state.prefetch_queue.remove(idx);
                        let meta = WallpaperMetadata::from(prefetched);
                        let mut engine = WallpaperEngine::new(&app_cfg, &mut state, &cache_mgr, &provider_mgr);
                        match engine.set_and_record_wallpaper(meta, true).await {
                            Ok(()) => {
                                let mut refill_cfg = app_cfg.clone();
                                if refill_cfg.manager.prefetch_count < 5 {
                                    refill_cfg.manager.prefetch_count = 5;
                                }
                                let _ = prefetch::refill_prefetch_queue(&mut state, &refill_cfg, &provider_mgr, &cache_mgr).await;
                                let _ = state.save();
                                let _ = ui_sender.send(UiUpdate::StateUpdated {
                                    state: state.clone(),
                                    status: "Planned wallpaper applied".into(),
                                }).await;
                            }
                            Err(e) => {
                                let _ = ui_sender.send(UiUpdate::Error(format!("Failed to apply planned wallpaper: {}", e))).await;
                            }
                        }
                    }
                }
                Action::RefillQueue => {
                    let _ = ui_sender.send(UiUpdate::StatusMessage("Refilling prefetch queue...".into())).await;
                    let mut refill_cfg = app_cfg.clone();
                    if refill_cfg.manager.prefetch_count < 5 {
                        refill_cfg.manager.prefetch_count = 5;
                    }
                    let _ = prefetch::refill_prefetch_queue(&mut state, &refill_cfg, &provider_mgr, &cache_mgr).await;
                    let _ = state.save();
                    let _ = ui_sender.send(UiUpdate::StateUpdated {
                        state: state.clone(),
                        status: "Prefetch queue updated".into(),
                    }).await;
                }
                Action::ReloadConfig(new_cfg) => {
                    app_cfg = new_cfg;
                    let _ = ui_sender.send(UiUpdate::StatusMessage("Updating queue for new config...".into())).await;
                    let mut refill_cfg = app_cfg.clone();
                    if refill_cfg.manager.prefetch_count < 5 {
                        refill_cfg.manager.prefetch_count = 5;
                    }
                    let _ = prefetch::refill_prefetch_queue(&mut state, &refill_cfg, &provider_mgr, &cache_mgr).await;
                    let _ = state.save();
                    let _ = ui_sender.send(UiUpdate::StateUpdated {
                        state: state.clone(),
                        status: "Settings updated".into(),
                    }).await;
                }
            }
        }
    });

    BackendHandle { sender: action_tx }
}
