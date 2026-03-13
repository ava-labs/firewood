#!/usr/bin/env bash
set -euo pipefail

if command -v just &> /dev/null; then
    exec just "$@"
else
    echo "Error: 'just' is not installed." >&2
    echo "" >&2
    echo "To install just:" >&2
    echo "  - Visit: https://github.com/casey/just#installation" >&2
    echo "  - Or use cargo: cargo install just" >&2
    exit 1
fi
