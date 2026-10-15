pub mod cache;
pub mod config;
pub mod engine;
#[cfg(feature = "sandbox-proxy")]
pub mod http_proxy;
pub mod monitor;
pub mod prefetch;
pub mod provider;
pub mod sources;
pub mod state;
pub mod wallpaper;

wasmtime::component::bindgen!({
    path: "../../wit",
    world: "wallpaper-provider",
    exports: { default: async },
});
