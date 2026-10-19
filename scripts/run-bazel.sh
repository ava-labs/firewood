#!/usr/bin/env bash

set -euo pipefail

if command -v bazelisk &> /dev/null; then
    exec bazelisk "$@"
elif command -v nix &> /dev/null; then
    exec nix run nixpkgs#bazelisk -- "$@"
else
    echo "Error: Neither bazelisk nor nix is installed." >&2
    echo "" >&2
    echo "Please install one of the following:" >&2
    echo "" >&2
    echo "Option 1 - Install Bazelisk:" >&2
    echo "  - Visit: https://github.com/bazelbuild/bazelisk#installation" >&2
    echo "" >&2
    echo "Option 2 - Install Nix:" >&2
    echo "  - Visit: https://nixos.org/download/" >&2
    echo "  - Or run (multi-user install): curl -L https://nixos.org/nix/install | sh -s -- --daemon" >&2
    exit 1
fi
