# Development Environment

Firewood is a Rust workspace (edition 2024, minimum supported Rust 1.94.0) with a Go
FFI layer. This guide covers three ways to set up a development environment and how to
verify it.

## Common prerequisites

- **Rust** via [rustup](https://rustup.rs/), with the pinned toolchain and the
  `rustfmt`, `clippy`, and `rust-analyzer` components.
- **[just](https://github.com/casey/just)** — the command runner used throughout this
  repository (`just --list` shows available recipes).
- **Go** (see `.devcontainer/Dockerfile` and `ffi/go.mod` for the version) — required
  for the FFI layer.
- **[Nix](https://nixos.org/)** — used by the FFI flake (`ffi/flake.nix`).
- **The mdBook toolchain** for building this site: `mdbook`, `mdbook-mermaid`,
  `mdbook-linkcheck2`. Install with
  `cargo binstall mdbook@0.5.3 mdbook-mermaid@0.17.0 mdbook-linkcheck2@0.12.2`.

## macOS (local)

1. Install rustup and the components:

   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   rustup component add rustfmt clippy rust-analyzer
   ```

2. Install `just` and the mdBook toolchain (e.g. via `brew install just` and the
   `cargo binstall` line above).
3. In VS Code, install the `rust-analyzer` extension and point it at the workspace.
4. Build and test:

   ```bash
   cargo build
   cargo nextest run --workspace --features ethhash,logger --all-targets
   ```

## Docker / dev container

Use the checked-in `.devcontainer/`:

1. Open the repository in VS Code with the **Dev Containers** extension and choose
   "Reopen in Container". The container builds from `.devcontainer/Dockerfile`.
2. The container's `postCreateCommand` reports the toolchain state
   (`rustup show && go version && nix --version && sccache --show-stats`).
3. Rust, Go, Nix, and `sccache` are preinstalled; run the build/test commands from the
   macOS section inside the container.

## Remote Linux over SSH

1. Connect with the VS Code **Remote-SSH** extension; `rust-analyzer` runs on the host.
2. On the host, install rustup, Go, and Nix as in the common prerequisites.
3. To preview this site, run `just book-serve` on the host and forward the mdBook port
   (3000) to your machine (VS Code forwards it automatically, or use
   `ssh -L 3000:localhost:3000 <host>`).

## Verifying your setup

Run the same checks CI runs (see `CONTRIBUTING.md`):

```bash
cargo fmt
cargo nextest run --workspace --features ethhash,logger --all-targets
cargo clippy --workspace --features ethhash,logger --all-targets
cargo doc --no-deps
```

All tests must pass with no clippy warnings. To build this documentation site
locally, run `just book-build` (or `just book-serve` for live reload).
