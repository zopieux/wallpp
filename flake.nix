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
      lib = nixpkgs.lib;

      rustVersion = "1.99.0";

      # Rust toolchain (with the wasm32-wasip2 target that providers
      # compile to) and rustPlatform for one system. Shared by the
      # builtin packages and by buildProvider below.
      # wasm32-wasip2 is Tier 2 since Rust 1.82, so stable suffices.
      mkToolchain =
        system:
        let
          pkgs = import nixpkgs {
            inherit system;
            overlays = [ (import rust-overlay) ];
          };
          rustToolchain = pkgs.rust-bin.stable.${rustVersion}.default.override {
            targets = [ "wasm32-wasip2" ];
            extensions = [ "rust-src" ];
          };
        in
        {
          inherit pkgs rustToolchain;
          rustPlatform = pkgs.makeRustPlatform {
            cargo = rustToolchain;
            rustc = rustToolchain;
          };
        };

      # Build a wallpp provider crate to a WebAssembly component and
      # install it as $out/share/wallpp/providers/<name>.wasm.
      #
      # Exposed for third-party provider flakes, for example:
      #
      #   packages.${system}.default = wallpp.buildProvider {
      #     inherit system;
      #     name = "other";
      #     src = ./.;
      #     cargoLock = ./Cargo.lock;
      #   };
      #
      # The provider crate is expected to be named
      # "wallpp-provider-<name>" (override with `crateName`) and to
      # depend on `wallpp-provider-sdk`. Git dependencies need their
      # content hash supplied via `outputHashes` (nix cannot guess it):
      #
      #   outputHashes = {
      #     "wallpp-provider-sdk-0.1.0" = "sha256-…";
      #   };
      buildProvider =
        {
          system,
          name,
          src,
          cargoLock,
          version ? "0.1.0",
          crateName ? "wallpp-provider-${name}",
          outputHashes ? { },
        }:
        let
          inherit (mkToolchain system) rustPlatform;
          wasmArtifact = "${lib.replaceStrings [ "-" ] [ "_" ] crateName}.wasm";
        in
        rustPlatform.buildRustPackage {
          pname = crateName;
          inherit version src;
          cargoLock = {
            lockFile = cargoLock;
            inherit outputHashes;
          };
          CARGO_BUILD_TARGET = "wasm32-wasip2";
          buildPhase = ''
            runHook preBuild
            cargo build -j $NIX_BUILD_CORES --profile release --target wasm32-wasip2 --offline -p ${crateName}
            runHook postBuild
          '';
          doCheck = false;
          installPhase = ''
            mkdir -p $out/share/wallpp/providers
            cp target/wasm32-wasip2/release/${wasmArtifact} $out/share/wallpp/providers/${name}.wasm
          '';
        };
    in
    {
      inherit buildProvider;

      packages = forAllSystems (
        system:
        let
          inherit (mkToolchain system) pkgs rustPlatform;
          providerNames = [
            "reddit"
            "spotlight"
            "unsplash"
            "wallhaven"
          ];

          # Build all internal providers in a single derivation for speed.
          providers = rustPlatform.buildRustPackage {
            pname = "wallpp-providers";
            version = "0.1.0";
            src = ./.;
            cargoLock.lockFile = ./Cargo.lock;
            CARGO_BUILD_TARGET = "wasm32-wasip2";
            buildPhase = ''
              runHook preBuild
              cargo build -j $NIX_BUILD_CORES --profile release --target wasm32-wasip2 --offline \
                ${lib.concatMapStringsSep " " (name: "-p wallpp-provider-${name}") providerNames}
              runHook postBuild
            '';
            doCheck = false;
            installPhase = ''
              mkdir -p $out/share/wallpp/providers
              ${lib.concatMapStringsSep "\n" (
                name:
                let
                  crateName = "wallpp-provider-${name}";
                  wasmArtifact = "${lib.replaceStrings [ "-" ] [ "_" ] crateName}.wasm";
                in
                "cp target/wasm32-wasip2/release/${wasmArtifact} $out/share/wallpp/providers/${name}.wasm"
              ) providerNames}
            '';
          };

          # Individual provider packages extracted from the combined build.
          providerNamedPackages = lib.genAttrs (map (name: "wallpp-provider-${name}") providerNames) (
            pkgName:
            let
              name = lib.removePrefix "wallpp-provider-" pkgName;
            in
            pkgs.runCommand "${pkgName}-0.1.0" { } ''
              mkdir -p $out/share/wallpp/providers
              cp ${providers}/share/wallpp/providers/${name}.wasm $out/share/wallpp/providers/
            ''
          );

          wallpp-bin = rustPlatform.buildRustPackage {
            pname = "wallpp-bin";
            version = "0.1.0";
            src = ./.;
            cargoLock.lockFile = ./Cargo.lock;
            cargoBuildFlags = [
              "-p"
              "wallpp"
              "-p"
              "wallpp-gtk"
            ];
            cargoTestFlags = [
              "-p"
              "wallpp"
              "-p"
              "wallpp-gtk"
            ];
            nativeBuildInputs = [
              pkgs.pkg-config
              pkgs.wrapGAppsHook4
            ];
            buildInputs = with pkgs; [
              gtk4
              libxcb
              libxrandr
              wayland
              gsettings-desktop-schemas
            ];
          };

          wallpp = pkgs.symlinkJoin {
            name = "wallpp-0.1.0";
            paths = [
              wallpp-bin
              providers
            ];
            nativeBuildInputs = [ pkgs.makeWrapper ];
            postBuild = ''
              wrapProgram $out/bin/wallpp \
                --prefix XDG_DATA_DIRS : "$out/share" \
                --prefix PATH : "$out/bin:${pkgs.lib.makeBinPath [ pkgs.procps ]}" \
                --prefix LD_LIBRARY_PATH : "${
                  pkgs.lib.makeLibraryPath (
                    with pkgs;
                    [
                      libxcb
                      libxrandr
                      wayland
                    ]
                  )
                }"
              wrapProgram $out/bin/wallpp-gtk \
                --prefix XDG_DATA_DIRS : "$out/share"
            '';
            meta.mainProgram = "wallpp";
          };

          wallpp-gtk = pkgs.symlinkJoin {
            name = "wallpp-gtk-0.1.0";
            paths = [ wallpp ];
            meta.mainProgram = "wallpp-gtk";
          };
        in
        {
          inherit
            wallpp-bin
            wallpp
            wallpp-gtk
            providers
            ;
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
          rustToolchain = pkgs.rust-bin.stable.${rustVersion}.default.override {
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
              export LD_LIBRARY_PATH="${
                pkgs.lib.makeLibraryPath (
                  with pkgs;
                  [
                    libxcb
                    libxrandr
                    wayland
                    gtk4
                  ]
                )
              }''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
              export XDG_DATA_DIRS="${pkgs.gsettings-desktop-schemas}/share/gsettings-schemas/${pkgs.gsettings-desktop-schemas.name}:$XDG_DATA_DIRS"
              TARGET_DIR="''${CARGO_TARGET_DIR:-$(cargo metadata --format-version 1 --no-deps 2>/dev/null | jq -r .target_directory 2>/dev/null || echo "$PWD/target")}"
              export WALLPP_PROVIDERS_DIR="$TARGET_DIR/wasm32-wasip2/debug:$TARGET_DIR/wasm32-wasip2/release"
            '';
          };
        }
      );
    };
}
