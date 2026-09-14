#!/usr/bin/env bash
# shellcheck disable=SC2034
# Shared Firewood toolchain versions for provisioned Linux environments.
#
# Keep this file side-effect free: provisioning scripts source it to avoid
# drifting Rust, Go, and developer-tool pins across on-prem sessions,
# benchmark hosts, and other scripted environments.

FIREWOOD_RUSTUP_VERSION=1.29.1
FIREWOOD_RUSTUP_X86_64_LINUX_GNU_SHA256=dda7234360b7f578ca8b0ddcb80145646fa61a67c1720a5abc7051b35c9fcb71
FIREWOOD_RUST_VERSION=1.94.1
FIREWOOD_RUST_NIGHTLY=nightly-2026-09-13

FIREWOOD_GO_VERSION=1.26.0
FIREWOOD_GO_LINUX_AMD64_SHA256=aac1b08a0fb0c4e0a7c1555beb7b59180b05dfc5a3d62e40e9de90cd42f88235

FIREWOOD_CARGO_BINSTALL_VERSION=1.21.1
FIREWOOD_CARGO_BINSTALL_X86_64_LINUX_MUSL_SHA256=630c8f8803a686aa6779497f0f0fb51d49822fb5fc3c514d8ced33b34e338e6e

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

FIREWOOD_GO_TOOLS=(
	github.com/reteps/dockerfmt@v0.5.4
	mvdan.cc/sh/v3/cmd/shfmt@v3.14.1
)
