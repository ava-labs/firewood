# Development Environment

Firewood is a Rust workspace (edition 2024, minimum supported Rust 1.94.0) with a Go
FFI layer. This guide covers three ways to set up a development environment and how to
verify it.

## Common prerequisites

- **Rust** via [rustup](https://rustup.rs/), with the `rustfmt`, `clippy`,
  `rust-analyzer`, and `rust-src` components, plus the pinned nightly toolchain
  (`nightly-2026-07-05`) that the `clippy` checks use (see
  [Verifying your setup](#verifying-your-setup)).
- **[just](https://github.com/casey/just)** — the command runner used throughout this
  repository (`just --list` shows available recipes).
- **Go** (see `ffi/go.mod` for the version) — required
  for the FFI layer.
- **[Nix](https://nixos.org/)** — used by the FFI flake (`ffi/flake.nix`).
- **[jq](https://jqlang.github.io/jq/)** — used by the in-repo mdBook frontmatter
  preprocessor, so `just book-build` and `book-serve` work on a fresh machine.
- **The mdBook toolchain** for building this site: `mdbook`, `mdbook-mermaid`,
  `mdbook-linkcheck2`. Install with
  `cargo binstall mdbook@0.5.3 mdbook-mermaid@0.17.0`. Install `mdbook-linkcheck2`
  separately: `cargo binstall mdbook-linkcheck2@0.12.2` (it is not in the
  `taiki-e/install-action` manifest, so CI installs it via `cargo binstall` directly).

## macOS (local)

1. Install rustup, the stable components, and the pinned nightly used for clippy:

   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   rustup component add rustfmt clippy rust-analyzer rust-src
   rustup toolchain install nightly-2026-07-05 --component clippy
   ```

2. Install `just` and the mdBook toolchain (for example, via `brew install just` and the
   `cargo binstall` line above).
3. In VS Code, install the `rust-analyzer` extension and point it at the workspace.
4. Build and test:

   ```bash
   cargo build
   cargo nextest run --workspace --features ethhash,logger --all-targets
   ```

## Docker / dev container

Requires a running Docker daemon (Docker Desktop on macOS, Docker Engine on Linux)
and the VS Code **Dev Containers** extension. Use the checked-in `.devcontainer/` (see
[`.devcontainer/README.md`](https://github.com/ava-labs/firewood/blob/main/.devcontainer/README.md)):

1. Open the repository in VS Code with the **Dev Containers** extension and run
   **"Reopen in Container"**.
2. The container is assembled entirely from published Dev Container features plus a
   local `firewood-tools` feature over an Ubuntu 26.04 base — no separate Dockerfile.
   Rust, Go, Nix, `sccache`, and the project's developer tooling are all provided.
3. `post-create.sh` (the `postCreateCommand`) fixes volume ownership and runs a
   verification smoke test (`rustup show && go version && nix --version &&
   sccache --show-stats`). `postStartCommand` also runs `sccache --show-stats` on
   every container start to confirm the cache daemon is live.
4. Developer authentication (`gh`, Claude Code) and shell history persist across
   rebuilds via named volumes — log in once. Then run the build/test commands from the
   macOS section inside the container.

> [!IMPORTANT]
> If you previously used the old Dockerfile-based container, upgrading requires a
> one-time **rebuild**. "Reopen in Container" reuses the stale container and fails
> with `unable to find user vscode`. Run **"Dev Containers: Rebuild Container"** instead.

For the design behind this setup, see
[Development Container](../designs/0004-devcontainer.md).

## Remote Linux over SSH

1. Connect with the VS Code **Remote-SSH** extension; `rust-analyzer` runs on the host.
2. On the host, install rustup, Go, and Nix as in the common prerequisites.
3. To preview this site, run `just book-serve` on the host and forward the mdBook port
   (3000) to your machine (VS Code forwards it automatically, or use
   `ssh -L 3000:localhost:3000 <host>`).

## Verifying your setup

Verify your environment by running the same checks CI runs (see `CONTRIBUTING.md`):

```bash
cargo fmt
cargo nextest run --workspace --features ethhash,logger --all-targets
cargo +nightly-2026-07-05 clippy --workspace --features ethhash,logger --all-targets
cargo +nightly-2026-07-05 clippy --profile maxperf --workspace --features ethhash,logger --all-targets
cargo doc --no-deps
```

All tests must pass with no clippy warnings. To build this documentation site
locally, run `just book-build` (or `just book-serve` for live reload).
