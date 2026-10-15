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
    #[cfg(feature = "sandbox-proxy")]
    fn send_request(
        &mut self,
        request: hyper::Request<wasmtime_wasi_http::body::HyperOutgoingBody>,
        config: wasmtime_wasi_http::types::OutgoingRequestConfig,
    ) -> wasmtime_wasi_http::HttpResult<wasmtime_wasi_http::types::HostFutureIncomingResponse> {
        crate::http_proxy::send_request(request, config)
    }
}

#[derive(Clone)]
pub struct DiscoveredProvider {
    pub name: String,
    pub path: PathBuf,
    pub info: ProviderInfo,
    /// Compiled component, cached so list/download/validate calls do not
    /// re-JIT-compile the provider `.wasm` on every invocation.
    pub component: Component,
}

impl std::fmt::Debug for DiscoveredProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DiscoveredProvider")
            .field("name", &self.name)
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
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
        // Enable Wasmtime's on-disk compilation cache so repeated
        // `Component::from_file` calls across runs do not re-JIT-compile
        // every provider from scratch (tens of seconds in debug builds).
        if let Ok(cache) = wasmtime::Cache::from_file(None) {
            cfg.cache(Some(cache));
        }
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

        // Explicit override via environment variable.
        if let Ok(paths) = std::env::var("WALLPP_PROVIDERS_DIR") {
            for p in paths.split(':') {
                if !p.is_empty() {
                    dirs.push(PathBuf::from(p));
                }
            }
        }

        // User XDG data directory (~/.local/share/wallpp/providers).
        dirs.push(crate::paths::data_dir().join("wallpp").join("providers"));

        // System XDG directories.
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

    fn discovered(&self, provider_name: &str) -> Result<&DiscoveredProvider> {
        self.providers
            .get(provider_name)
            .with_context(|| format!("Provider '{}' not found in search paths", provider_name))
    }

    async fn instantiate(
        &self,
        component: &Component,
        memory_mb: usize,
    ) -> Result<(Store<HostState>, WallpaperProvider)> {
        let mut store = Store::new(
            &self.engine,
            HostState {
                wasi: WasiCtxBuilder::new().build(),
                http: WasiHttpCtx::new(),
                table: ResourceTable::new(),
                limits: StoreLimitsBuilder::new()
                    .memory_size(memory_mb << 20)
                    .build(),
            },
        );
        store.limiter(|s| &mut s.limits);
        let instance =
            WallpaperProvider::instantiate_async(&mut store, component, &self.linker).await?;
        Ok((store, instance))
    }

    async fn load_info_from_file(&mut self, path: &Path) -> Result<()> {
        let component = Component::from_file(&self.engine, path)?;
        let (mut store, instance) = self.instantiate(&component, 64).await?;
        let guest = instance.wallpp_provider_provider();
        let info = guest.call_info(&mut store).await?;

        self.providers.insert(
            info.name.clone(),
            DiscoveredProvider {
                name: info.name.clone(),
                path: path.to_path_buf(),
                info,
                component,
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
        let discovered = self.discovered(provider_name)?;
        let (mut store, instance) = self.instantiate(&discovered.component, 128).await?;
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
        let discovered = self.discovered(provider_name)?;
        let (mut store, instance) = self.instantiate(&discovered.component, 128).await?;
        let guest = instance.wallpp_provider_provider();
        let image = guest
            .call_download(&mut store, &cfg, id)
            .await?
            .map_err(|e| anyhow::anyhow!("Provider download error: {:?}", e))?;

        Ok(image)
    }

    pub async fn validate_config(&self, provider_name: &str, cfg: WitConfig) -> Result<WitConfig> {
        let discovered = self.discovered(provider_name)?;
        let (mut store, instance) = self.instantiate(&discovered.component, 64).await?;
        let guest = instance.wallpp_provider_provider();
        let validated = guest
            .call_validate_config(&mut store, &cfg)
            .await?
            .map_err(|e| {
                anyhow::anyhow!("Invalid config for provider '{}': {:?}", provider_name, e)
            })?;

        Ok(validated)
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
