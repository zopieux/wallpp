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
# Build the reddit provider (debug)
cargo build -p wallpp-provider-reddit --target wasm32-wasip2

# Or build in release mode
cargo build -p wallpp-provider-reddit --target wasm32-wasip2 --release
```

The generated `.wasm` file lands in `$CARGO_TARGET_DIR/wasm32-wasip2/{debug,release}/` where `wallpp` will automatically discover it.

### Run the manager CLI

```bash
# Inspect all provider search paths and their status
cargo run -p wallpp -- search-paths

# Inspect discovered providers and their schema / options
cargo run -p wallpp -- providers

# Query and list wallpapers from configured sources
cargo run -p wallpp -- list

# Pick next wallpaper, download it into cache, and print details
cargo run -p wallpp -- next
```

## Configuration

`wallpp` reads configuration from `$XDG_CONFIG_HOME/wallpp/config.toml` or the path pointed to by `$WALLPP_CONFIG`.

If no configuration file exists, `wallpp` uses a default Reddit source (wallpapers subreddit, hot sort).

### Example `config.toml`

```toml
# Reddit provider instance 1
[[source]]
name = "nature"
provider = "reddit"
subreddits = ["EarthPorn", "wallpapers"]
sort = "top"
time = "week"

# Reddit provider instance 2
[[source]]
name = "space"
provider = "reddit"
subreddits = ["spaceporn"]
sort = "hot"
```

To run against a specific source:

```bash
cargo run -p wallpp -- list --source nature
cargo run -p wallpp -- next --source space
```

## Create a new provider

1. Create a crate under `providers/<name>` with `crate-type = ["cdylib"]`.
2. Depend on `wallpp-provider-sdk = { path = "../../crates/wallpp-provider-sdk" }` and `wstd`.
3. Implement the `Guest` trait (`info()`, `list()`, `download()`).
4. Export the provider via `wallpp_provider_sdk::export!(YourProvider with_types_in wallpp_provider_sdk);`.
5. Build with `cargo build -p wallpp-provider-<name> --target wasm32-wasip2`.
