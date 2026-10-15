{
  description = "wallpp, a minimal wallpaper manager";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs?ref=nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      rust-overlay,
    }:
    let
      forAllSystems = nixpkgs.lib.genAttrs nixpkgs.lib.systems.flakeExposed;
      providerNames = [
        "reddit"
        "windows"
        "unsplash"
        "wallhaven"
      ];
    in
    {
      packages = forAllSystems (
        system:
        let
          pkgs = import nixpkgs {
            inherit system;
            overlays = [ (import rust-overlay) ];
          };
          lib = nixpkgs.lib;
          rustToolchain = pkgs.rust-bin.stable.latest.default.override {
            targets = [ "wasm32-wasip2" ];
            extensions = [ "rust-src" ];
          };
          rustPlatform = pkgs.makeRustPlatform {
            cargo = rustToolchain;
            rustc = rustToolchain;
          };

          mkProvider =
            name:
            let
              crateName = "wallpp-provider-${name}";
              wasmArtifact = "${builtins.replaceStrings [ "-" ] [ "_" ] crateName}.wasm";
            in
            rustPlatform.buildRustPackage {
              pname = crateName;
              version = "0.1.0";
              src = ./.;
              cargoLock.lockFile = ./Cargo.lock;
              buildPhase = ''
                runHook preBuild
                cargo build -j $NIX_BUILD_CORES --profile release --target wasm32-wasip2 --offline -p ${crateName}
                runHook postBuild
              '';
              checkPhase = ":";
              installPhase = ''
                mkdir -p $out/share/wallpp/providers
                cp target/wasm32-wasip2/release/${wasmArtifact} $out/share/wallpp/providers/${name}.wasm
              '';
            };

          providerPkgs = lib.genAttrs providerNames mkProvider;
          providerNamedPackages = lib.mapAttrs' (
            name: pkg: lib.nameValuePair "wallpp-provider-${name}" pkg
          ) providerPkgs;

          wallpp-bin = rustPlatform.buildRustPackage {
            pname = "wallpp-bin";
            version = "0.1.0";
            src = ./.;
            cargoLock.lockFile = ./Cargo.lock;
            cargoBuildFlags = [
              "-p"
              "wallpp"
            ];
            nativeBuildInputs = [ pkgs.pkg-config ];
            buildInputs = with pkgs; [
              libxcb
              libxrandr
              wayland
            ];
          };

          wallpp = pkgs.symlinkJoin {
            name = "wallpp-0.1.0";
            paths = [ wallpp-bin ] ++ (builtins.attrValues providerPkgs);
            nativeBuildInputs = [ pkgs.makeWrapper ];
            postBuild = ''
              wrapProgram $out/bin/wallpp \
                --prefix XDG_DATA_DIRS : "$out/share" \
                --prefix PATH : "${pkgs.lib.makeBinPath [ pkgs.procps ]}" \
                --prefix LD_LIBRARY_PATH : "${pkgs.lib.makeLibraryPath (with pkgs; [ libxcb libxrandr wayland ])}"
            '';
            meta.mainProgram = "wallpp";
          };
        in
        {
          inherit wallpp-bin wallpp;
          default = wallpp;
        }
        // providerNamedPackages
      );

      devShells = forAllSystems (
        system:
        let
          pkgs = import nixpkgs {
            inherit system;
            overlays = [ (import rust-overlay) ];
          };
          rustToolchain = pkgs.rust-bin.stable.latest.default.override {
            targets = [ "wasm32-wasip2" ];
            extensions = [
              "rust-src"
              "rustfmt"
              "clippy"
            ];
          };
        in
        {
          default = pkgs.mkShell {
            RUST_SRC_PATH = "${rustToolchain}/lib/rustlib/src/rust/library";
            buildInputs = with pkgs; [
              rustToolchain
              pkg-config
              procps
              jq
              libxcb
              libxrandr
              wayland
              gtk4
              gsettings-desktop-schemas
            ];
            shellHook = ''
              export LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath (with pkgs; [ libxcb libxrandr wayland gtk4 ])}''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
              export XDG_DATA_DIRS="${pkgs.gsettings-desktop-schemas}/share/gsettings-schemas/${pkgs.gsettings-desktop-schemas.name}:$XDG_DATA_DIRS"
              TARGET_DIR="''${CARGO_TARGET_DIR:-$(cargo metadata --format-version 1 --no-deps 2>/dev/null | jq -r .target_directory 2>/dev/null || echo "$PWD/target")}"
              export WALLPP_PROVIDERS_DIR="$TARGET_DIR/wasm32-wasip2/debug:$TARGET_DIR/wasm32-wasip2/release"
            '';
          };
        }
      );
    };
}
