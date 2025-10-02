{
  # To use:
  #  - Install nix: https://github.com/DeterminateSystems/nix-installer?tab=readme-ov-file#install-nix
  #  - Run from ffi/ dir: `nix build .#firewood-ffi`
  #  - Run from anywhere: `nix develop 'github:ava-labs/firewood?dir=ffi&ref=[SHA]'`

  description = "Firewood FFI library and development environment";

  inputs = {
    nixpkgs.url = "https://flakehub.com/f/NixOS/nixpkgs/0.2505.*.tar.gz";
    rust-overlay.url = "github:oxalica/rust-overlay";
    crane.url = "github:ipetkov/crane";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, rust-overlay, crane, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
    let
      overlays = [ (import rust-overlay) ];
      pkgs = import nixpkgs { inherit system overlays; };
      inherit (pkgs) lib;

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
          (craneLib.filterCargoSources path type);
      };

      commonArgs = {
        inherit src;
        strictDeps = true;
        dontStrip = true;

        # Build only the firewood-ffi crate
        pname = ffiCargoToml.package.name;
        version = workspaceCargoToml.workspace.package.version;

        CARGO_PROFILE = "maxperf";

        nativeBuildInputs = with pkgs; [
          pkg-config
        ];
      } // lib.optionalAttrs pkgs.stdenv.isDarwin {
        # Set macOS deployment target for Darwin builds
        MACOSX_DEPLOYMENT_TARGET = "13.0";
      };

      cargoArtifacts = craneLib.buildDepsOnly (commonArgs // {
        cargoExtraArgs = "--package ${ffiCargoToml.package.name} --features ethhash,logger";
      });

      firewood-ffi = craneLib.buildPackage (commonArgs // {
        inherit cargoArtifacts;
        cargoExtraArgs = "--package ${ffiCargoToml.package.name} --features ethhash,logger --frozen";

        # Install the static library and header
        postInstall = ''
          mkdir -p $out/lib $out/include
          cp target/maxperf/libfirewood_ffi.a $out/lib/ || cp target/release/libfirewood_ffi.a $out/lib/
          cp ffi/firewood.h $out/include/
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

      devShells.default = craneLib.devShell {
        inputsFrom = [ firewood-ffi ];

        packages = with pkgs; [
          rustToolchain
          go
        ];

        shellHook = ''
          echo "Firewood FFI development environment"
          echo "Use 'nix build .#firewood-ffi' to build the FFI package"

          # Set up CGO environment
          export CGO_LDFLAGS="-L${firewood-ffi}/lib -lfirewood_ffi -lm"
          export CGO_CFLAGS="-I${firewood-ffi}/include"

          # TODO(marun) Maybe add a script?
          echo ""
          echo "To run Go tests:"
          echo "  export GOEXPERIMENT=cgocheck2"
          echo "  export TEST_FIREWOOD_HASH_MODE=ethhash"
          echo "  go test ./... -v"
        '';
      };
    });
}
