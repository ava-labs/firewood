---
status: active
last-reviewed: 2026-06-17
source:
  - .devcontainer/
  - .github/workflows/devcontainer.yaml
---

# Development Container

## Overview

Firewood's development container provides the Rust and Go toolchains, Nix, and the
project's developer tooling. It is assembled declaratively from published
[Dev Container features](https://containers.dev/features) plus one local feature —
there is no `Dockerfile`. `devcontainer.json` layers features over the
`mcr.microsoft.com/devcontainers/base:ubuntu26.04` base image (which provides the
non-root `vscode` user, sudo, and common packages). Named Docker volumes persist
developer authentication and shell history across container rebuilds.

## Feature composition

`devcontainer.json` composes these features, each pinned to its major version so
patch and minor updates flow automatically while majors remain a deliberate bump:

| Source | Provides |
| --- | --- |
| `mcr.microsoft.com/devcontainers/base:ubuntu26.04` (image) | base OS, non-root `vscode` user, sudo, common packages |
| `github-cli:1` | `gh` |
| `go:1` | Go toolchain (tracks `latest`) and `golangci-lint` |
| `rust:1` (`profile: default`) | rustup + the stable toolchain with `clippy`, `rustfmt`, `rust-analyzer`, `rust-src` |
| `nix:1` | Nix in daemon mode (`experimental-features = nix-command flakes`, `trusted-users = root vscode`) |
| `node:1` | Node, required by the Claude Code feature |
| `docker-in-docker:2` (`moby: false`) | Docker CE — Moby's `moby-cli` is not published for Ubuntu 26.04 |
| `anthropics/devcontainer-features/claude-code:1` | the Claude Code CLI and its editor extension |
| `./features/firewood-tools` (local) | build deps, sccache, cargo-nextest and other Cargo/Go tools, shell wiring (starship, sccache config, Claude Code symlink) — see [The local `firewood-tools` feature](#the-local-firewood-tools-feature) |

Go tracks `latest` rather than a pinned version; `latest` always satisfies the Go
floor in `ffi/go.mod`, which ends the manual version tracking that previously drifted.

## The local `firewood-tools` feature

`.devcontainer/features/firewood-tools/` is an in-repo feature that installs tooling
with no maintained upstream equivalent. Its `devcontainer-feature.json` declares
`installsAfter: [rust, go]` so it runs after those features during the image build.
`install.sh` runs as root at build time and provides:

- Build dependencies: `clang`, `cmake`, `libssl-dev`, `pkg-config`, `build-essential`,
  `jq`, `shellcheck`.
- Extra stable-toolchain components (`llvm-tools`, `rust-docs`) and a full `nightly`
  toolchain.
- `cargo-binstall`, then `sccache`, then sccache wiring written into
  `/usr/local/cargo/config.toml`.
- Cargo tools (via binstall): `ast-grep`, `cargo-edit`, `cargo-expand`,
  `cargo-machete`, `cargo-msrv`, `cargo-nextest`, `git-cliff`, `just`, `ripgrep`,
  `rustfilt`, `starship`.
- Go tools (via `go install`): `dockerfmt`, `shfmt`.
- Shell wiring: `starship` init, a persistent `HISTFILE`, and a symlink from
  `~/.claude.json` to `~/.claude/user-claude.json` so Claude Code settings
  land in the persistent volume.

## Why there is no Dockerfile

When `devcontainer.json` uses `build.dockerfile` together with `features`, the CLI
builds the Dockerfile first and layers features on top — so anything that depends on
`cargo`/`rustup` (cargo-binstall, sccache, the cargo tools, the nightly toolchain)
cannot live in a Dockerfile, because it would run before the `rust` feature installs
the toolchain. The local feature's `installsAfter: [rust, go]` orders that tooling
after the toolchains within the feature layer, which is what makes the Dockerfile-less
layout work. `installsAfter` only *orders* features that are already listed; it does
not pull missing ones in, so `rust` and `go` stay explicitly listed in
`devcontainer.json`.

## Persistent state

Named volumes (suffixed `-${localEnv:USER}` for per-user isolation on shared hosts)
start empty and fill on first use, so a one-time login survives rebuilds:

| Volume target | Purpose |
| --- | --- |
| `/home/vscode/.cache/sccache` | sccache cache |
| `/usr/local/cargo/registry`, `/usr/local/cargo/git` | cargo caches |
| `/go/pkg` | Go module cache |
| `/home/vscode/.config/gh` | `gh` authentication |
| `/home/vscode/.claude` | Claude Code authentication, projects, and history |
| `/home/vscode/.commandhistory` | shell history |

Cargo and Go homes keep the features' defaults (`/usr/local/cargo`,
`/usr/local/rustup`, `GOPATH=/go`); the cache volumes mount onto sub-paths so the
features' PATH entries and tool proxies stay intact.

## Build and runtime behavior

- The container starts as the base image's `vscode` user.
- Named volumes are created root-owned and mounted after the image build, so
  `post-create.sh` (the `postCreateCommand`) `chown`s the volume targets to `vscode`
  by username, then runs a verification smoke test (`rustup show`, `go version`,
  `nix --version`, `sccache --show-stats`). The script is idempotent and safe to re-run on
  rebuild.
- `postStartCommand` runs `sccache --show-stats`.

## Trade-offs and known interactions

- **Rebuild required when upgrading** from the old Dockerfile-based container: an
  editor reuses a container by its workspace-folder label, so the upgrade needs a
  one-time "Rebuild Container" or the stale container fails with
  `unable to find user vscode`.
- The container user changes from the host `$USER` to `vscode`.
- Go floats at `latest` rather than a pinned version.
- Nix uses the upstream daemon-mode installer with `/nix` on a volume; the `ffi/`
  flake is unchanged.
- **IPv6 and `claude login`:** disabling IPv6 is not done by default (it is hard to
  reverse). If `claude login` fails, `devcontainer.json` carries a commented
  `--sysctl net.ipv6.conf.all.disable_ipv6=1` runArg to uncomment; see
  [`.devcontainer/README.md`](https://github.com/ava-labs/firewood/blob/main/.devcontainer/README.md).

## Testing

`.github/workflows/devcontainer.yaml` builds the container with the `devcontainers/ci`
action on every `.devcontainer/**` change and smoke-tests the toolchains and
feature-provided tools (`cargo +nightly`, `golangci-lint`, `shfmt`, `dockerfmt`,
`starship`, `claude`, `shellcheck`, `jq`, and more). Because the action runs the full
create lifecycle, `post-create.sh` is exercised in CI.

## Related designs

See [Design Documents](../README.md) for the design workflow. For the broader local setup, see
[Development Environment](../../getting-started/dev-environment.md).
