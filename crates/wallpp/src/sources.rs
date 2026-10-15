use std::collections::HashMap;

use crate::config::AppConfig;
use crate::provider::DiscoveredProvider;

static EMPTY_OPTIONS: std::sync::LazyLock<HashMap<String, toml::Value>> =
    std::sync::LazyLock::new(HashMap::new);

/// Computed or configured source with its associated discovered provider.
#[derive(Debug, Clone)]
pub struct ResolvedSource<'a> {
    /// Friendly display label (e.g. "Reddit" or "Reddit #2" or custom name).
    pub display_label: String,
    /// Explicit user-configured custom name, if provided.
    pub custom_name: Option<String>,
    /// Provider runtime metadata and operations.
    pub provider: &'a DiscoveredProvider,
    /// Configuration options for this source.
    pub options: &'a HashMap<String, toml::Value>,
}

/// Computes the display label for a source given its provider's label, an optional custom name,
/// and the occurrence index among sources using the same provider.
pub fn compute_display_label(
    provider_label: &str,
    custom_name: Option<&str>,
    occurrence_index: usize,
) -> String {
    if let Some(name) = custom_name {
        let trimmed = name.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    if occurrence_index == 0 {
        provider_label.to_string()
    } else {
        format!("{} #{}", provider_label, occurrence_index + 1)
    }
}

/// Resolves active sources based on configuration and discovered providers.
/// When `app_cfg.source` is empty, all discovered providers are enabled by default.
/// When `source_filter` is specified, filters by source label or provider name.
pub fn resolve_sources<'a>(
    app_cfg: &'a AppConfig,
    source_filter: Option<&str>,
    discovered: &'a HashMap<String, DiscoveredProvider>,
) -> Vec<ResolvedSource<'a>> {
    let mut resolved = Vec::new();

    if app_cfg.source.is_empty() {
        // Default mode: all discovered providers enabled with default options.
        let mut sorted_keys: Vec<_> = discovered.keys().collect();
        sorted_keys.sort();
        for key in sorted_keys {
            let prov = &discovered[key];
            resolved.push(ResolvedSource {
                display_label: prov.info.label.clone(),
                custom_name: None,
                provider: prov,
                options: &EMPTY_OPTIONS,
            });
        }
    } else {
        let mut provider_counts: HashMap<&str, usize> = HashMap::new();
        for src in &app_cfg.source {
            if let Some(prov) = discovered.get(&src.provider) {
                let occurrence = provider_counts.entry(&src.provider).or_insert(0);
                let label =
                    compute_display_label(&prov.info.label, src.name.as_deref(), *occurrence);
                *occurrence += 1;

                resolved.push(ResolvedSource {
                    display_label: label,
                    custom_name: src.name.clone(),
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
    }

    if let Some(filter) = source_filter {
        let filtered: Vec<_> = resolved
            .into_iter()
            .filter(|s| {
                s.display_label.eq_ignore_ascii_case(filter)
                    || s.custom_name
                        .as_deref()
                        .is_some_and(|n| n.eq_ignore_ascii_case(filter))
                    || s.provider.name.eq_ignore_ascii_case(filter)
            })
            .collect();

        if !filtered.is_empty() {
            return filtered;
        }

        // Direct fallback: if filter explicitly targets a discovered provider name.
        for (name, prov) in discovered {
            if name.eq_ignore_ascii_case(filter) || prov.info.label.eq_ignore_ascii_case(filter) {
                return vec![ResolvedSource {
                    display_label: prov.info.label.clone(),
                    custom_name: None,
                    provider: prov,
                    options: &EMPTY_OPTIONS,
                }];
            }
        }

        Vec::new()
    } else {
        resolved
    }
}
