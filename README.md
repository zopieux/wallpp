# wallpp

A minimal wallpaper manager with a sandboxed WASM provider plugin system (`wasm32-wasip2` via Wasmtime).

## Developing your own provider

Implement the `Guest` trait from the SDK in a `cdylib` crate named `wallpp-provider-<name>` (`info()`, `list()`, `download()`, `validate-config()`). Depend on the SDK with a cargo git dependency on this repository; cargo locates the SDK crate inside it:

```toml
[dependencies]
wallpp-provider-sdk = { git = "https://github.com/zopieux/wallpp", rev = "…" }
```

Ship it as a flake with one call (see `buildProvider` in `flake.nix`):

```nix
{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    wallpp.url = "github:zopieux/wallpp";
  };
  outputs = { self, nixpkgs, wallpp }:
    let system = "x86_64-linux"; in {
      packages.${system}.default = wallpp.buildProvider {
        inherit system;
        name = "other";
        src = ./.;
        cargoLock = ./Cargo.lock;
        outputHashes = {
          "wallpp-provider-sdk-0.1.0" = "sha256-…"; # nix prints the value on first build
        };
      };
    };
}
```

During development, point a local wallpp at your build output with `WALLPP_PROVIDERS_DIR=$PWD/target/wasm32-wasip2/debug`.

## Usage with third-party providers

Install the provider package alongside wallpp; both install their `.wasm` files to `$out/share/wallpp/providers/`, which NixOS merges into `XDG_DATA_DIRS`, and wallpp discovers every provider there at startup:

```nix
{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    wallpp.url = "github:zopieux/wallpp";
    wallpp-other.url = "github:3p/wallpp-other";
    wallpp-other.inputs.nixpkgs.follows = "nixpkgs";
    wallpp-other.inputs.wallpp.follows = "wallpp";  # Reuse toolchain.
  };
  outputs = { self, nixpkgs, wallpp, wallpp-other, ... }:
    let system = "x86_64-linux"; in {
      nixosConfigurations.myhost = nixpkgs.lib.nixosSystem {
        inherit system;
        modules = [{
          environment.systemPackages = [
            wallpp.packages.${system}.wallpp
            wallpp-other.packages.${system}.default
          ];
        }];
      };
    };
}
```

Provider names must be unique: discovery keys providers by `info().name` and a duplicate silently replaces the earlier one.

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

# Or build in release mode:
cargo build-providers --release
```

The generated `.wasm` files land in `$CARGO_TARGET_DIR/wasm32-wasip2/{debug,release}/` where `wallpp` will automatically discover them.

### Run the manager CLI

```bash
# Run as resident background service (default on-boot and interval refresh)
cargo run -p wallpp

# Pick next wallpaper, download it, and apply it to the desktop
cargo run -p wallpp -- next

# Switch to the previous wallpaper in history
cargo run -p wallpp -- previous

# Preview next wallpaper (downloads to cache and prints details without setting)
cargo run -p wallpp -- preview

# Open the GTK configuration editor (execs the wallpp-gtk binary)
cargo run -p wallpp -- config

# Inspect the currently set wallpaper (metadata, URL, path)
cargo run -p wallpp -- info

# Inspect all provider search paths and their status
cargo run -p wallpp -- search-paths
```

## Configuration

`wallpp` reads configuration from `$XDG_CONFIG_HOME/wallpp/config.toml` or the path pointed to by `$WALLPP_CONFIG`.

If no configuration file exists, `wallpp` automatically treats all discovered providers (`Reddit`, `Spotlight`, etc.) as available sources with their default options.

### Example `config.toml`

```toml
# Reddit provider instance
[[source]]
name = "nature"
provider = "reddit"
subreddits = ["EarthPorn", "wallpapers"]
sort = "top"
time = "week"
```

To run against a specific source:

```bash
cargo run -p wallpp -- next --source nature
cargo run -p wallpp -- preview --source nature
```

## Adding a built-in provider

1. Create a crate under `providers/<name>` with `crate-type = ["cdylib"]`.
2. Depend on `wallpp-provider-sdk = { path = "../../crates/wallpp-provider-sdk" }` and `wstd`.
3. Implement the `Guest` trait (`info()`, `list()`, `download()`, `validate-config()`).
4. Export the provider via `wallpp_provider_sdk::export!(YourProvider with_types_in wallpp_provider_sdk);`.
5. Add `<name>` to `providerNames` in `flake.nix` so the flake builds it.
6. Build with `cargo build -p wallpp-provider-<name> --target wasm32-wasip2`.
