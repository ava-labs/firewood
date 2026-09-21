#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=infra/toolchains/firewood-toolchain.sh
. "$SCRIPT_DIR/../../infra/toolchains/firewood-toolchain.sh"

RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}"
CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"
export RUSTUP_HOME CARGO_HOME
export PATH="$CARGO_HOME/bin:$PATH"

curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --no-modify-path --profile default \
        --default-toolchain "$FIREWOOD_RUST_VERSION"

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
