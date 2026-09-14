#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=infra/toolchains/firewood-toolchain.sh
. "$SCRIPT_DIR/../../infra/toolchains/firewood-toolchain.sh"

RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}"
CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"
export RUSTUP_HOME CARGO_HOME
export PATH="$CARGO_HOME/bin:$PATH"

TMP_DIR="$(mktemp -d)"
cleanup() {
	rm -rf "$TMP_DIR"
}
trap cleanup EXIT

download_checked() {
	local url="$1"
	local sha256="$2"
	local output="$3"

	curl -fsSL "$url" -o "$output"
	printf '%s  %s\n' "$sha256" "$output" | sha256sum -c -
}

UNAME_OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
UNAME_ARCH="$(uname -m)"

if [ "$UNAME_OS" != linux ] || [ "$UNAME_ARCH" != x86_64 ]; then
	echo "Unsupported platform: $UNAME_OS/$UNAME_ARCH" >&2
	echo "Pinned benchmark Rust install currently supports linux/x86_64." >&2
	exit 1
fi

rustup_init="$TMP_DIR/rustup-init"
download_checked \
	"https://static.rust-lang.org/rustup/archive/${FIREWOOD_RUSTUP_VERSION}/x86_64-unknown-linux-gnu/rustup-init" \
	"$FIREWOOD_RUSTUP_X86_64_LINUX_GNU_SHA256" \
	"$rustup_init"
chmod 0755 "$rustup_init"
"$rustup_init" -y --no-modify-path --profile default \
	--default-toolchain "$FIREWOOD_RUST_VERSION"

rustup component add llvm-tools rust-docs --toolchain "$FIREWOOD_RUST_VERSION"
rustup toolchain install "$FIREWOOD_RUST_NIGHTLY" --profile minimal
rustup component add clippy rustfmt rust-src rust-docs llvm-tools \
	--toolchain "$FIREWOOD_RUST_NIGHTLY"

PROFILE="$HOME/.profile"
if ! grep -qF "$CARGO_HOME/bin" "$PROFILE" 2>/dev/null; then
	cat >>"$PROFILE" <<EOF

# Firewood benchmark Rust toolchain
export RUSTUP_HOME="$RUSTUP_HOME"
export CARGO_HOME="$CARGO_HOME"
export PATH="\$CARGO_HOME/bin:\$PATH"
EOF
fi

rustup show
