use anyhow::{Context, Result};
use std::collections::HashSet;
use std::path::PathBuf;

use crate::cache::CacheManager;
use crate::config::AppConfig;
use crate::provider::ProviderManager;
use crate::state::{current_utc_timestamp, PrefetchedWallpaper, State};

/// A cached wallpaper and its reduced-resolution grid copy.
#[derive(Debug, Clone)]
pub struct CachedImage {
    pub original_path: PathBuf,
    pub thumbnail_path: PathBuf,
}

/// Return the cached image for `id`, downloading and caching it on a miss.
/// Thumbnail generation runs in a blocking worker so image decoding never
/// stalls the async provider/download tasks. Shared by the prefetch refill
/// and the GTK source previews.
pub async fn cached_image(
    provider_mgr: &ProviderManager,
    cache_mgr: &CacheManager,
    provider: &str,
    cfg: &crate::wallpp::provider::types::Config,
    id: &str,
) -> Result<CachedImage> {
    if let Some(original_path) = cache_mgr.get_cached_image(id) {
        let expected_thumbnail = cache_mgr.thumbnail_path(&original_path);
        if expected_thumbnail.is_file() {
            return Ok(CachedImage {
                original_path,
                thumbnail_path: expected_thumbnail,
            });
        }
        let cache = cache_mgr.clone();
        let original_for_task = original_path.clone();
        let thumbnail_path =
            tokio::task::spawn_blocking(move || cache.ensure_thumbnail(&original_for_task))
                .await
                .unwrap_or_else(|_| original_path.clone());
        return Ok(CachedImage {
            original_path,
            thumbnail_path,
        });
    }
    let img = provider_mgr
        .query_download(provider, cfg.clone(), id)
        .await?;
    let cache = cache_mgr.clone();
    let id_owned = id.to_owned();
    let original_path = tokio::task::spawn_blocking(move || {
        cache.store_image(&id_owned, &img.content_type, &img.data)
    })
    .await
    .context("Thumbnail/cache task failed")??;
    let thumbnail_path = cache_mgr.thumbnail_path(&original_path);
    Ok(CachedImage {
        thumbnail_path: if thumbnail_path.is_file() {
            thumbnail_path
        } else {
            original_path.clone()
        },
        original_path,
    })
}

pub async fn refill_prefetch_queue(
    state: &mut State,
    config: &AppConfig,
    provider_mgr: &ProviderManager,
    cache_mgr: &CacheManager,
    on_progress: &mut (dyn FnMut(&State) + Send),
) -> Result<usize> {
    // Retain only prefetched items whose files exist on disk.
    state.prefetch_queue.retain(|item| item.cache_path.exists());

    let target_count = config.manager.prefetch_count;
    if state.prefetch_queue.len() >= target_count {
        return Ok(0);
    }

    let needed = target_count - state.prefetch_queue.len();
    let mut downloaded = 0;

    // Track IDs already in history or already prefetched to avoid duplicates.
    let mut seen_ids: HashSet<String> = state
        .history
        .iter()
        .map(|h| h.opaque_id.clone())
        .chain(state.prefetch_queue.iter().map(|p| p.opaque_id.clone()))
        .collect();

    // Determine sources to pick from.
    let sources = crate::sources::resolve_sources(config, None, &provider_mgr.providers);
    if sources.is_empty() {
        return Ok(0);
    }

    let mut source_idx = crate::sources::pick_source_index(
        &sources,
        config.manager.source_strategy,
        state.last_source(),
    );
    let max_attempts = needed * 3;
    let mut attempts = 0;

    while state.prefetch_queue.len() < target_count && attempts < max_attempts {
        attempts += 1;
        let src = &sources[source_idx % sources.len()];
        source_idx += 1;

        let wit_cfg = crate::config::toml_to_wit_config(
            src.options,
            &src.provider.info.options,
            &src.provider.info.default_config,
        );

        let filter = crate::monitor::compute_filter_criteria(config.manager.min_display_percentage);

        let page = match provider_mgr
            .query_list(&src.provider.name, wit_cfg.clone(), 10, None, filter)
            .await
        {
            Ok(p) => p,
            Err(e) => {
                eprintln!(
                    "Warning: failed to query provider '{}' for prefetch: {}",
                    src.provider.name, e
                );
                continue;
            }
        };

        for item in page.items {
            if seen_ids.contains(&item.id) {
                continue;
            }

            // Reuse an already cached download instead of fetching the
            // same image again (list results are often stable across runs).
            let cached = match cached_image(
                provider_mgr,
                cache_mgr,
                &src.provider.name,
                &wit_cfg,
                &item.id,
            )
            .await
            {
                Ok(p) => Some(p),
                Err(e) => {
                    eprintln!(
                        "Warning: prefetch download failed for item from '{}': {}",
                        src.provider.name, e
                    );
                    None
                }
            };

            if let Some(cached) = cached {
                seen_ids.insert(item.id.clone());
                state.prefetch_queue.push(PrefetchedWallpaper {
                    provider: src.provider.name.clone(),
                    source_name: Some(src.display_label.clone()),
                    opaque_id: item.id,
                    source_url: item.source_url,
                    cache_path: cached.original_path,
                    title: item.title,
                    author: item.author,
                    width: item.width,
                    height: item.height,
                    prefetched_at: current_utc_timestamp(),
                });
                downloaded += 1;
                // Let callers (e.g. the GTK backend) publish each new
                // planned wallpaper immediately instead of waiting for
                // the whole queue to fill.
                on_progress(state);
                if state.prefetch_queue.len() >= target_count {
                    break;
                }
            }
        }
    }

    if downloaded > 0 {
        state.save()?;
    }

    Ok(downloaded)
}
