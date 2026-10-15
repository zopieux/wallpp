pub mod cache;
pub mod config;
pub mod engine;
#[cfg(feature = "sandbox-proxy")]
pub mod http_proxy;
pub mod monitor;
pub mod paths;
pub mod prefetch;
pub mod provider;
pub mod sources;
pub mod state;
pub mod wallpaper;

wasmtime::component::bindgen!({
    path: "../wallpp-provider-sdk/wit",
    world: "wallpaper-provider",
    exports: { default: async },
});
