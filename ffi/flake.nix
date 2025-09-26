{
  description = "Firewood FFI library and development environment";

  inputs = {
    nixpkgs.url = "https://flakehub.com/f/NixOS/nixpkgs/0.2505.*.tar.gz";
    rust-overlay.url = "github:oxalica/rust-overlay";
  };

  outputs = { self, nixpkgs, rust-overlay }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin" ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
    in
    {
      packages = forAllSystems (system:
        let
          overlays = [ (import rust-overlay) ];
          pkgs = import nixpkgs { inherit system overlays; };
          rustToolchain = pkgs.rust-bin.stable.latest.default.override {
            extensions = [ "rust-src" "rustfmt" "clippy" ];
          };
        in
        {
          firewood-ffi = pkgs.rustPlatform.buildRustPackage {
            pname = "firewood-ffi";
            version = "0.0.12";

            src = ./..;

            cargoLock = {
              lockFile = ../Cargo.lock;
            };

            # Build only the firewood-ffi crate
            cargoBuildFlags = [ "--package" "firewood-ffi" ];
            cargoTestFlags = [ "--package" "firewood-ffi" ];

            nativeBuildInputs = with pkgs; [
              rustToolchain
              pkg-config
            ] ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [
              pkgs.darwin.apple_sdk.frameworks.Security
              pkgs.darwin.apple_sdk.frameworks.SystemConfiguration
            ];

            buildInputs = with pkgs; [
              gcc
            ];

            # Install the static library and header
            postInstall = ''
              mkdir -p $out/lib $out/include
              cp target/*/libfirewood_ffi.a $out/lib/ || cp target/release/libfirewood_ffi.a $out/lib/
              cp ffi/firewood.h $out/include/
            '';

            meta = with pkgs.lib; {
              description = "C FFI bindings for Firewood, an embedded key-value store";
              homepage = "https://github.com/ava-labs/firewood";
              license = licenses.free;
              platforms = platforms.all;
            };
          };


          default = self.packages.${system}.firewood-ffi;
        });

      devShells = forAllSystems (system:
        let
          overlays = [ (import rust-overlay) ];
          pkgs = import nixpkgs { inherit system overlays; };
          rustToolchain = pkgs.rust-bin.stable.latest.default.override {
            extensions = [ "rust-src" "rustfmt" "clippy" ];
          };
        in
        {
          default = pkgs.mkShell {
            packages = with pkgs; [
              rustToolchain
              pkg-config
              gcc
            ] ++ pkgs.lib.optionals (!pkgs.stdenv.isDarwin) [
              # Linux-specific packages
            ] ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [
              # Darwin-specific packages
              pkgs.darwin.apple_sdk.frameworks.Security
              pkgs.darwin.apple_sdk.frameworks.SystemConfiguration
            ];

            shellHook = ''
              echo "Firewood FFI development environment"
              echo "Use 'nix build .#firewood-ffi' to build the FFI package"
              echo "Built package will have headers in include/ and library in lib/"
            '';
          };
        });
    };
}