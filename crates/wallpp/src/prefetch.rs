use anyhow::Result;
use std::collections::HashSet;

use crate::cache::CacheManager;
use crate::config::{AppConfig, SourceConfig};
use crate::provider::ProviderManager;
use crate::state::{current_utc_timestamp, PrefetchedWallpaper, State};

pub async fn refill_prefetch_queue(
    state: &mut State,
    config: &AppConfig,
    provider_mgr: &mut ProviderManager,
    cache_mgr: &CacheManager,
) -> Result<usize> {
    // 1. Retain only prefetched items whose files exist on disk.
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
    let sources = get_available_sources(config, provider_mgr);
    if sources.is_empty() {
        return Ok(0);
    }

    let mut source_idx = match config.manager.source_strategy {
        crate::config::SourceStrategy::Random => {
            let seed = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            (seed as usize) % sources.len()
        }
        crate::config::SourceStrategy::RoundRobin => {
            if let Some(last) = state.last_source() {
                if let Some(pos) = sources.iter().position(|(name, prov, _)| {
                    name.as_deref().is_some_and(|n| n.eq_ignore_ascii_case(last))
                        || prov.eq_ignore_ascii_case(last)
                }) {
                    (pos + 1) % sources.len()
                } else {
                    0
                }
            } else {
                0
            }
        }
    };
    let max_attempts = needed * 3;
    let mut attempts = 0;

    while state.prefetch_queue.len() < target_count && attempts < max_attempts {
        attempts += 1;
        let (source_name, provider_name, source_cfg) = &sources[source_idx % sources.len()];
        source_idx += 1;

        let Some(discovered) = provider_mgr.providers.get(provider_name) else {
            continue;
        };

        let wit_cfg = if let Some(sc) = source_cfg {
            crate::config::toml_to_wit_config(
                &sc.options,
                &discovered.info.options,
                &discovered.info.default_config,
            )
        } else {
            discovered.info.default_config.clone()
        };

        let filter = crate::wallpp::provider::types::FilterCriteria {
            min_width: None,
            min_height: None,
            orientation: None,
        };

        let page = match provider_mgr
            .query_list(provider_name, wit_cfg.clone(), 10, None, filter)
            .await
        {
            Ok(p) => p,
            Err(e) => {
                eprintln!(
                    "Warning: failed to query provider '{}' for prefetch: {}",
                    provider_name, e
                );
                continue;
            }
        };

        for item in page.items {
            if seen_ids.contains(&item.id) {
                continue;
            }

            match provider_mgr
                .query_download(provider_name, wit_cfg.clone(), &item.id)
                .await
            {
                Ok(img) => {
                    let cache_path =
                        cache_mgr.store_image(&item.id, &img.content_type, &img.data)?;
                    seen_ids.insert(item.id.clone());
                    state.prefetch_queue.push(PrefetchedWallpaper {
                        provider: provider_name.clone(),
                        source_name: source_name.clone(),
                        opaque_id: item.id,
                        source_url: item.source_url,
                        cache_path,
                        title: item.title,
                        author: item.author,
                        width: item.width,
                        height: item.height,
                        prefetched_at: current_utc_timestamp(),
                    });
                    downloaded += 1;
                    if state.prefetch_queue.len() >= target_count {
                        break;
                    }
                }
                Err(e) => {
                    eprintln!(
                        "Warning: prefetch download failed for item from '{}': {}",
                        provider_name, e
                    );
                }
            }
        }
    }

    if downloaded > 0 {
        state.save()?;
    }

    Ok(downloaded)
}

fn get_available_sources(
    config: &AppConfig,
    provider_mgr: &ProviderManager,
) -> Vec<(Option<String>, String, Option<SourceConfig>)> {
    if !config.source.is_empty() {
        config
            .source
            .iter()
            .filter(|s| provider_mgr.providers.contains_key(&s.provider))
            .map(|s| (s.name.clone(), s.provider.clone(), Some(s.clone())))
            .collect()
    } else {
        provider_mgr
            .providers
            .keys()
            .map(|p| (None, p.clone(), None))
            .collect()
    }
}
