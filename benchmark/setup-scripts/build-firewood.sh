#!/bin/bash
set -o errexit

if [ "$EUID" -eq 0 ]; then
    echo "This script should be run as a non-root user"
    exit 1
fi

# install rust
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
bash "$SCRIPT_DIR/install-rust.sh"
# shellcheck source=/dev/null
. "$HOME/.cargo/env"

# clone the firewood repository
if [ ! -d "$HOME/firewood" ]; then
    mkdir -p "$HOME/firewood"
fi
pushd "$HOME/firewood"

git clone https://github.com/ava-labs/firewood.git .

# build the firewood binary
cargo build --profile maxperf
popd
