{
  # To test with arbitrary firewood versions (alternative to firewood-go-ethhash):
  #  - Install nix: https://github.com/DeterminateSystems/nix-installer?tab=readme-ov-file#install-nix
  #  - Clone firewood locally at desired version/commit
  #  - Build: `cd ffi && nix build`
  #  - In your Go project: `go mod edit -replace github.com/ava-labs/firewood-go-ethhash/ffi=/path/to/firewood/ffi/result/ffi`

  description = "Firewood FFI library and development environment";

  inputs = {
    nixpkgs.url = "https://flakehub.com/f/NixOS/nixpkgs/0.2505.*.tar.gz";
    rust-overlay.url = "github:oxalica/rust-overlay";
    crane.url = "github:ipetkov/crane";
    flake-utils.url = "github:numtide/flake-utils";
    golang.url = "github:ava-labs/avalanchego?dir=nix/go&ref=f10757d594eedf0f016bc1400739788c542f005f";
  };

  outputs = { self, nixpkgs, rust-overlay, crane, flake-utils, golang }:
    flake-utils.lib.eachDefaultSystem (system:
    let
      overlays = [ (import rust-overlay) ];
      pkgs = import nixpkgs { inherit system overlays; };
      inherit (pkgs) lib;

      go = golang.packages.${system}.default;

      rustToolchain = pkgs.rust-bin.stable.latest.default.override {
        extensions = [ "rust-src" "rustfmt" "clippy" ];
      };

      craneLib = (crane.mkLib pkgs).overrideToolchain rustToolchain;

      # Extract crate info from Cargo.toml files
      ffiCargoToml = builtins.fromTOML (builtins.readFile ./Cargo.toml);
      workspaceCargoToml = builtins.fromTOML (builtins.readFile ../Cargo.toml);

      src = lib.cleanSourceWith {
        src = craneLib.path ./..;
        filter = path: type:
          (lib.hasSuffix "\.md" path) ||
          (lib.hasSuffix "\.go" path) ||
          (lib.hasSuffix "go.mod" path) ||
          (lib.hasSuffix "go.sum" path) ||
          (lib.hasSuffix "firewood.h" path) ||
          (craneLib.filterCargoSources path type);
      };

      # Map Nix platform to Rust target triple (Go/Rust use aarch64, Nix uses arm64)
      rustTarget = {
        "aarch64-darwin" = "aarch64-apple-darwin";
        "x86_64-darwin" = "x86_64-apple-darwin";
        "aarch64-linux" = "aarch64-unknown-linux-gnu";
        "x86_64-linux" = "x86_64-unknown-linux-gnu";
      }.${pkgs.stdenv.hostPlatform.system} or pkgs.stdenv.hostPlatform.config;

      commonArgs = {
        inherit src;
        strictDeps = true;
        dontStrip = true;

        # Build only the firewood-ffi crate
        pname = ffiCargoToml.package.name;
        version = workspaceCargoToml.workspace.package.version;

        nativeBuildInputs = with pkgs; [
          pkg-config
        ];

      } // lib.optionalAttrs pkgs.stdenv.isDarwin {
        # Set macOS deployment target for Darwin builds
        MACOSX_DEPLOYMENT_TARGET = "15.0";
      };

      mkFfi = { cargoCmd, targetProfile, extraArgs ? {} }:
        craneLib.buildPackage (commonArgs // {
          cargoArtifacts = craneLib.buildDepsOnly (commonArgs // {
            cargoBuildCommand = cargoCmd;
          } // extraArgs);
          cargoBuildCommand = cargoCmd;
          doCheck = false;
          postInstall = ''
            mkdir -p $out/ffi
            cp -R ./ffi/* $out/ffi/
            mkdir -p $out/ffi/libs/${rustTarget}
            cp target/${targetProfile}/libfirewood_ffi.a $out/ffi/libs/${rustTarget}/
            cd $out/ffi
            HOME=$TMPDIR GOTOOLCHAIN=local FIREWOOD_LD_MODE=STATIC_LIBS ${go}/bin/go generate
          '';
          meta = with lib; {
            description = "C FFI bindings for Firewood, an embedded key-value store";
            homepage = "https://github.com/ava-labs/firewood";
            license = {
              fullName = "Ava Labs Ecosystem License 1.1";
              url = "https://github.com/ava-labs/firewood/blob/main/LICENSE.md";
            };
            platforms = [ "x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin" ];
          };
        } // extraArgs);

      firewood-ffi = mkFfi {
        cargoCmd = "cargo build-static-ffi";
        targetProfile = "maxperf";
      };

      firewood-ffi-profile = mkFfi {
        cargoCmd = "cargo build-profile-ffi";
        targetProfile = "profiling";
        extraArgs.RUSTFLAGS = "-C force-frame-pointers=yes";
      };
    in
    {
      packages = {
        inherit firewood-ffi firewood-ffi-profile;
        default = firewood-ffi;
        profile = firewood-ffi-profile;
      };

      apps.go = {
        type = "app";
        program = "${go}/bin/go";
      };

      apps.jq = {
        type = "app";
        program = "${pkgs.jq}/bin/jq";
      };

      apps.just = {
        type = "app";
        program = "${pkgs.just}/bin/just";
      };

      devShells.default = craneLib.devShell {
        inputsFrom = [ firewood-ffi ];

        packages = with pkgs; [
          firewood-ffi
          go
          jq
          just
          rustToolchain
          # Profiling tools
          rustfilt
          inferno
          jemalloc
          graphviz
          perl
        ];

        shellHook = ''
          # Ensure golang bin is in the path
          GOBIN="$(go env GOPATH)/bin"
          if [[ ":$PATH:" != *":$GOBIN:"* ]]; then
            export PATH="$GOBIN:$PATH"
          fi

          # Force sequential build of vendored jemalloc for reproducibility
          export MAKEFLAGS="-j1"
        '';
      };
    });
}
