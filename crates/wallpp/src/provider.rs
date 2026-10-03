use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use wasmtime::{
    component::{Component, Linker, ResourceTable},
    Config, Engine, Store, StoreLimits, StoreLimitsBuilder,
};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};
use wasmtime_wasi_http::{WasiHttpCtx, WasiHttpView};

use crate::wallpp::provider::types::{
    Config as WitConfig, FilterCriteria, Image, Orientation, Page, ProviderInfo,
};
use crate::WallpaperProvider;

pub struct HostState {
    pub wasi: WasiCtx,
    pub http: WasiHttpCtx,
    pub table: ResourceTable,
    pub limits: StoreLimits,
}

impl WasiView for HostState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

impl WasiHttpView for HostState {
    fn ctx(&mut self) -> &mut WasiHttpCtx {
        &mut self.http
    }
    fn table(&mut self) -> &mut ResourceTable {
        &mut self.table
    }
}

pub struct DiscoveredProvider {
    pub name: String,
    pub path: PathBuf,
    pub info: ProviderInfo,
}

pub struct ProviderManager {
    pub engine: Engine,
    pub linker: Linker<HostState>,
    pub providers: HashMap<String, DiscoveredProvider>,
}

impl ProviderManager {
    pub async fn new() -> Result<Self> {
        let mut cfg = Config::new();
        cfg.wasm_component_model(true).async_support(true);
        let engine = Engine::new(&cfg)?;

        let mut linker = Linker::<HostState>::new(&engine);
        wasmtime_wasi::p2::add_to_linker_async(&mut linker)?;
        wasmtime_wasi_http::add_only_http_to_linker_async(&mut linker)?;

        let mut mgr = Self {
            engine,
            linker,
            providers: HashMap::new(),
        };
        mgr.discover().await?;
        Ok(mgr)
    }

    pub fn search_dirs() -> Vec<PathBuf> {
        let mut dirs = Vec::new();

        // 1. Explicit override via environment variable.
        if let Ok(paths) = std::env::var("WALLPP_PROVIDERS_DIR") {
            for p in paths.split(':') {
                if !p.is_empty() {
                    dirs.push(PathBuf::from(p));
                }
            }
        }

        // 2. User XDG data directory (~/.local/share/wallpp/providers).
        let user_data = std::env::var("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
                PathBuf::from(home).join(".local").join("share")
            });
        dirs.push(user_data.join("wallpp").join("providers"));

        // 4. System XDG directories.
        let system_data = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| {
            "/usr/local/share:/usr/share:/run/current-system/sw/share".to_string()
        });
        for p in system_data.split(':') {
            if !p.is_empty() {
                dirs.push(PathBuf::from(p).join("wallpp").join("providers"));
            }
        }

        dirs
    }

    pub async fn discover(&mut self) -> Result<()> {
        let dirs = Self::search_dirs();
        for dir in dirs {
            if !dir.exists() {
                continue;
            }
            if let Ok(entries) = std::fs::read_dir(&dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().and_then(|ext| ext.to_str()) == Some("wasm") {
                        if let Err(e) = self.load_info_from_file(&path).await {
                            eprintln!("Warning: Failed to load provider from {:?}: {}", path, e);
                        }
                    }
                }
            }
        }
        Ok(())
    }

    async fn load_info_from_file(&mut self, path: &Path) -> Result<()> {
        let component = Component::from_file(&self.engine, path)?;
        let mut store = Store::new(
            &self.engine,
            HostState {
                wasi: WasiCtxBuilder::new().build(),
                http: WasiHttpCtx::new(),
                table: ResourceTable::new(),
                limits: StoreLimitsBuilder::new().memory_size(64 << 20).build(),
            },
        );
        store.limiter(|s| &mut s.limits);

        let instance =
            WallpaperProvider::instantiate_async(&mut store, &component, &self.linker).await?;
        let guest = instance.wallpp_provider_provider();
        let info = guest.call_info(&mut store).await?;

        self.providers.insert(
            info.name.clone(),
            DiscoveredProvider {
                name: info.name.clone(),
                path: path.to_path_buf(),
                info,
            },
        );

        Ok(())
    }

    pub async fn query_list(
        &self,
        provider_name: &str,
        cfg: WitConfig,
        limit: u32,
        cursor: Option<String>,
        filter: FilterCriteria,
    ) -> Result<Page> {
        let discovered = self
            .providers
            .get(provider_name)
            .with_context(|| format!("Provider '{}' not found in search paths", provider_name))?;

        let component = Component::from_file(&self.engine, &discovered.path)?;
        let mut store = Store::new(
            &self.engine,
            HostState {
                wasi: WasiCtxBuilder::new().build(),
                http: WasiHttpCtx::new(),
                table: ResourceTable::new(),
                limits: StoreLimitsBuilder::new().memory_size(128 << 20).build(),
            },
        );
        store.limiter(|s| &mut s.limits);

        let instance =
            WallpaperProvider::instantiate_async(&mut store, &component, &self.linker).await?;
        let guest = instance.wallpp_provider_provider();
        let mut page = guest
            .call_list(&mut store, &cfg, limit, cursor.as_deref(), filter)
            .await?
            .map_err(|e| anyhow::anyhow!("Provider list error: {:?}", e))?;

        // Centralized filtering across all providers.
        page.items.retain(|w| matches_filter(w, &filter));

        Ok(page)
    }

    pub async fn query_download(
        &self,
        provider_name: &str,
        cfg: WitConfig,
        id: &str,
    ) -> Result<Image> {
        let discovered = self
            .providers
            .get(provider_name)
            .with_context(|| format!("Provider '{}' not found in search paths", provider_name))?;

        let component = Component::from_file(&self.engine, &discovered.path)?;
        let mut store = Store::new(
            &self.engine,
            HostState {
                wasi: WasiCtxBuilder::new().build(),
                http: WasiHttpCtx::new(),
                table: ResourceTable::new(),
                limits: StoreLimitsBuilder::new().memory_size(128 << 20).build(),
            },
        );
        store.limiter(|s| &mut s.limits);

        let instance =
            WallpaperProvider::instantiate_async(&mut store, &component, &self.linker).await?;
        let guest = instance.wallpp_provider_provider();
        let image = guest
            .call_download(&mut store, &cfg, id)
            .await?
            .map_err(|e| anyhow::anyhow!("Provider download error: {:?}", e))?;

        Ok(image)
    }
}

pub fn matches_filter(
    w: &crate::wallpp::provider::types::Wallpaper,
    filter: &FilterCriteria,
) -> bool {
    if let Some(min_w) = filter.min_width {
        if let Some(w_val) = w.width {
            if w_val < min_w {
                return false;
            }
        }
    }
    if let Some(min_h) = filter.min_height {
        if let Some(h_val) = w.height {
            if h_val < min_h {
                return false;
            }
        }
    }
    if let Some(orientation) = filter.orientation {
        if let (Some(w_val), Some(h_val)) = (w.width, w.height) {
            match orientation {
                Orientation::Landscape if w_val <= h_val => {
                    return false;
                }
                Orientation::Portrait if h_val <= w_val => {
                    return false;
                }
                Orientation::Square if w_val != h_val => {
                    return false;
                }
                _ => {}
            }
        }
    }
    true
}
