pub mod cache;
pub mod config;
pub mod engine;
pub mod monitor;
pub mod prefetch;
pub mod provider;
pub mod sources;
pub mod state;
pub mod tui;
pub mod wallpaper;

wasmtime::component::bindgen!({
    path: "../../wit",
    world: "wallpaper-provider",
    exports: { default: async },
});
