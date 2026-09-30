#!/usr/bin/env bash
# Shared Firewood toolchain pins for provisioned Linux environments.
#
# Keep this file side-effect free: provisioning scripts source it so that Rust,
# Go and developer-tool versions do not drift between environments.
#
# It is also sourced from outside this repository, so the variable names below
# are an interface: renaming one, or dropping an entry because nothing here
# reads it, breaks a consumer this checkout cannot show you.
# FIREWOOD_CARGO_TOOLS and FIREWOOD_GO_TOOLS have no in-repo consumer today.
# The s5cmd pin is the exception -- .github/workflows/ci.yaml pins it too, and
# the two should agree.
#
# Every assignment here is read by a sourcing script, so SC2034 (unused
# variable) is disabled per entry rather than for the file.

# shellcheck disable=SC2034
FIREWOOD_RUST_VERSION=1.94.1

# Matches the `go` directive in ffi/go.mod, so provisioned hosts build the FFI
# with the toolchain CI tests it against. .github/workflows/verify-go-versions.yaml
# fails the build if the two diverge; the checksum comes from
# https://go.dev/dl/?mode=json&include=all.
# shellcheck disable=SC2034
FIREWOOD_GO_VERSION=1.25.10
# shellcheck disable=SC2034
FIREWOOD_GO_LINUX_AMD64_SHA256=42d4f7a32316aa66591eca7e89867256057a4264451aca10570a715b3637ba70

# shellcheck disable=SC2034
FIREWOOD_CARGO_BINSTALL_VERSION=1.21.1
# shellcheck disable=SC2034
FIREWOOD_CARGO_BINSTALL_X86_64_LINUX_MUSL_SHA256=630c8f8803a686aa6779497f0f0fb51d49822fb5fc3c514d8ced33b34e338e6e

# shellcheck disable=SC2034
FIREWOOD_CARGO_TOOLS=(
    ast-grep@0.45.3
    cargo-edit@0.13.13
    cargo-expand@1.0.126
    cargo-machete@0.9.2
    cargo-msrv@0.19.3
    cargo-nextest@0.9.144
    git-cliff@2.14.1
    just@1.58.0
    ripgrep@15.2.0
    rustfilt@0.2.1
    sccache@0.17.0
)

# shellcheck disable=SC2034
FIREWOOD_GO_TOOLS=(
    # avalanchego drives its build through a root Taskfile.yml, so anyone
    # working on the FFI against a local avalanchego needs this.
    github.com/go-task/task/v3/cmd/task@v3.53.1
    github.com/reteps/dockerfmt@v0.5.4
    # Moves C-Chain blocks and state snapshots to and from S3. The benchmark
    # workflow depends on it; .github/workflows/ci.yaml pins the same version.
    github.com/peak/s5cmd/v2@v2.3.0
    mvdan.cc/sh/v3/cmd/shfmt@v3.14.1
)
