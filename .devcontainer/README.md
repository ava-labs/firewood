# Firewood development container

A [Dev Container](https://containers.dev/) for Firewood. It provides the Rust
and Go toolchains, Nix, and the project's developer tooling.

## What provides what

Most of the environment is assembled from published Dev Container **features**:

- Base image: `mcr.microsoft.com/devcontainers/base:ubuntu26.04` (non-root
  `vscode` user, sudo, common packages).
- `github-cli`, `go` (+ golangci-lint), `rust`, `nix`, `node`,
  `docker-in-docker`, and the official Anthropic `claude-code` feature.
- A local `firewood-tools` feature (`features/firewood-tools/`) installs the
  Rust/Go tooling with no maintained upstream feature: `cargo-binstall`,
  `sccache`, `cargo-nextest` and other cargo extensions, the `nightly`
  toolchain, `dockerfmt`, `shfmt`, and `starship`.

## Persistent state

Named Docker volumes persist developer state across container rebuilds. Log in
once and the credentials survive a rebuild:

- `gh` auth — `~/.config/gh`
- Claude Code auth and history — `~/.claude` (settings file `~/.claude.json` is
  symlinked into this volume)
- Shell history — `~/.commandhistory`
- Build caches — sccache, the cargo registry/git caches, and the Go module
  cache.

## Troubleshooting

### `claude login` fails

In some networks `claude login` fails when IPv6 is enabled inside the
container. If that happens, uncomment the IPv6 line in the `runArgs` array of
`devcontainer.json` and rebuild:

```jsonc
"runArgs": [
  "--init",
  "--sysctl", "net.ipv6.conf.all.disable_ipv6=1"
]
```

This is left commented by default because disabling IPv6 is hard to reverse and
only some networks need it. See
[anthropics/devcontainer-features#24](https://github.com/anthropics/devcontainer-features/issues/24).
