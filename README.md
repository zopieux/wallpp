# wallpp

A minimal wallpaper manager with a sandboxed WASM provider plugin system (`wasm32-wasip2` via Wasmtime).

---

## Development workflow

```bash
nix develop
```

This shell provides `rustc`, `cargo` (with the `wasm32-wasip2` target installed), and automatically sets `WALLPP_PROVIDERS_DIR` to the Cargo target directory (`debug` and `release`).

### Build the provider plugins

Because providers compile as cdylib components for `wasm32-wasip2` while the manager binary compiles natively for your host machine, you must build the provider explicitly:

```bash
# Build all providers at once
cargo build-providers

# Or manually:
cargo build --workspace --exclude wallpp --target wasm32-wasip2

# Or build in release mode:
cargo build-providers --release
```

The generated `.wasm` files land in `$CARGO_TARGET_DIR/wasm32-wasip2/{debug,release}/` where `wallpp` will automatically discover them.

### Run the manager CLI

```bash
# Inspect all provider search paths and their status
cargo run -p wallpp -- search-paths

# Inspect discovered providers and their schema / options
cargo run -p wallpp -- providers

# Query and list wallpapers from configured sources
cargo run -p wallpp -- list

# Preview next wallpaper (downloads to cache and prints details without setting)
cargo run -p wallpp -- preview

# Pick next wallpaper, download it, and apply it to the desktop (default)
cargo run -p wallpp -- next
```

## Configuration

`wallpp` reads configuration from `$XDG_CONFIG_HOME/wallpp/config.toml` or the path pointed to by `$WALLPP_CONFIG`.

If no configuration file exists, `wallpp` automatically treats all discovered providers (`Reddit`, `Windows Spotlight`, etc.) as available sources with their default options.

### Example `config.toml`

```toml
# Reddit provider instance
[[source]]
name = "nature"
provider = "reddit"
subreddits = ["EarthPorn", "wallpapers"]
sort = "top"
time = "week"

# Windows Spotlight provider instance
[[source]]
name = "spotlight-4k"
provider = "windows"
locale = "en-US"
orientation = "landscape"

# Unsplash provider instance
[[source]]
name = "unsplash-nature"
provider = "unsplash"
orientation = "landscape"
query = "nature wallpapers"
# api_key = "..." # Optional override (defaults to built-in client ID)
```

To run against a specific source:

```bash
cargo run -p wallpp -- list --source nature
cargo run -p wallpp -- preview --source spotlight-4k
cargo run -p wallpp -- next --source spotlight-4k
```

## Create a new provider

1. Create a crate under `providers/<name>` with `crate-type = ["cdylib"]`.
2. Depend on `wallpp-provider-sdk = { path = "../../crates/wallpp-provider-sdk" }` and `wstd`.
3. Implement the `Guest` trait (`info()`, `list()`, `download()`).
4. Export the provider via `wallpp_provider_sdk::export!(YourProvider with_types_in wallpp_provider_sdk);`.
5. Build with `cargo build -p wallpp-provider-<name> --target wasm32-wasip2`.
