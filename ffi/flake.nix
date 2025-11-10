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

      # Fix Darwin platform config for CGO directives
      # Linux: pkgs.stdenv.hostPlatform.config already returns correct values (e.g. aarch64-unknown-linux-gnu)
      # Darwin: pkgs.stdenv.hostPlatform.config returns arm64-apple-darwin but we need aarch64-apple-darwin
      cgoHostPlatform = if pkgs.stdenv.isDarwin
        then builtins.replaceStrings ["arm64"] ["aarch64"] pkgs.stdenv.hostPlatform.config
        else pkgs.stdenv.hostPlatform.config;

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

        # Fix non-deterministic jemalloc builds on x86_64
        # Problem: tikv-jemalloc-sys vendors and builds jemalloc from source, which
        # uses -O3 optimization by default. This causes the symbol rtree_read.constprop.0
        # to appear inconsistently across builds, breaking reproducibility.
        # See: https://github.com/ava-labs/firewood/pull/1423
        #
        # Solution: Override jemalloc's -O3 with -O2 (compiler uses last flag)
        # This is the same fix applied to nixpkgs' system jemalloc package:
        # https://github.com/NixOS/nixpkgs/pull/393724 (o3-to-o2.patch)
        #
        # Note: The nixpkgs fix applies to the system jemalloc C library package
        # (pkgs/development/libraries/jemalloc), not to Rust's tikv-jemalloc-sys.
        # tikv-jemalloc-sys vendors its own jemalloc source, so we must apply
        # the fix here via CFLAGS, which tikv-jemalloc-sys passes to jemalloc's
        # configure script during the cargo build.
        # See: https://github.com/tikv/jemallocator/blob/master/jemalloc-sys/build.rs
        #
        # Note: As of this addition, tkiv-jemalloc-sys is the only C code in the
        # dependency tree. If this changes in the future, this flag may need to be
        # localized to only jemalloc builds.
        CFLAGS = "-O2";

        # Force sequential build of vendored jemalloc to avoid race conditions
        # MAKEFLAGS only affects make invocations (jemalloc), not cargo parallelism
        # See: https://github.com/NixOS/nixpkgs/issues/380852
        MAKEFLAGS = "-j1";

        # Enable jemalloc statistics for performance monitoring
        # This allows collection of allocation, active, resident, mapped, and metadata stats
        # Stats are automatically logged every 10 seconds via the metrics system
        JEMALLOC_SYS_WITH_MALLOC_CONF = "stats:true";
      } // lib.optionalAttrs (!pkgs.stdenv.hostPlatform.isAarch64) {
        # Set jemalloc page size to 4KB (2^12) for x86_64 reproducibility
        # Problem: On x86_64, jemalloc's autodetection can produce different page sizes
        # across builds, breaking reproducibility.
        # Solution: Explicitly set lg-page=12 (4KB) for x86_64, matching nixpkgs fix:
        # https://github.com/NixOS/nixpkgs/pull/393724
        # Note: aarch64 uses default page size (varies 4KB-16KB but is reproducible)
        JEMALLOC_SYS_WITH_LG_PAGE = "12";
      } // lib.optionalAttrs pkgs.stdenv.isDarwin {
        # Set macOS deployment target for Darwin builds
        MACOSX_DEPLOYMENT_TARGET = "13.0";
      };

      cargoArtifacts = craneLib.buildDepsOnly (commonArgs // {
        # Use cargo alias defined in .cargo/config.toml
        cargoBuildCommand = "cargo build-static-ffi";
      });

      firewood-ffi = craneLib.buildPackage (commonArgs // {
        inherit cargoArtifacts;
        # Use cargo alias defined in .cargo/config.toml
        cargoBuildCommand = "cargo build-static-ffi";

        # Disable tests - we only need to build the static library
        doCheck = false;

        # Install the static library and header
        postInstall = ''
          # Create a package structure compatible with FIREWOOD_LD_MODE=STATIC_LIBS
          mkdir -p $out/ffi
          cp -R ./ffi/* $out/ffi/
          mkdir -p $out/ffi/libs/${cgoHostPlatform}
          cp target/maxperf/libfirewood_ffi.a $out/ffi/libs/${cgoHostPlatform}/

          # Run go generate to switch CGO directives to STATIC_LIBS mode
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
      });
    in
    {
      packages = {
        inherit firewood-ffi;
        default = firewood-ffi;
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
