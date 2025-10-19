# List available recipes
default:
    @just --list

# Build ffi with nix
build-ffi-nix: check-nix
    cd ffi && nix build

# Check if the git branch is clean
check-clean-branch:
    #!/usr/bin/env bash
    set -euo pipefail

    git add --all
    git update-index --really-refresh >> /dev/null

    # Show the status of the working tree.
    git status --short

    # Exits if any uncommitted changes are found.
    git diff-index --quiet HEAD

# Check if the FFI flake (requires clean git tree)
check-ffi-flake-current: check-nix
    #!/usr/bin/env bash
    set -euo pipefail
    cd ffi
    nix flake update golang
    ../run-just.sh check-clean-branch

# Check if nix is installed
check-nix:
    #!/usr/bin/env bash
    set -euo pipefail
    if ! command -v nix &> /dev/null; then
        echo "Error: 'nix' is not installed." >&2
        echo "" >&2
        echo "To install nix:" >&2
        echo "  - Visit: https://github.com/DeterminateSystems/nix-installer" >&2
        echo "  - Or run: curl --proto '=https' --tlsv1.2 -sSf -L https://install.determinate.systems/nix | sh -s -- install" >&2
        exit 1
    fi

# Run all checks of ffi built with nix
test-ffi-nix: test-ffi-nix-build-equivalency test-ffi-nix-go-bindings

# Test ffi build equivalency between nix and cargo
test-ffi-nix-build-equivalency: check-nix
    #!/usr/bin/env bash
    set -euo pipefail

    echo "Testing ffi build equivalency between nix and cargo"

    bash -x ./ffi/test-build-equivalency.sh

# Test golang ffi bindings using the nix-built artifacts
test-ffi-nix-go-bindings: build-ffi-nix
    #!/usr/bin/env bash
    set -euo pipefail

    echo "Running ffi tests against bindings built by nix..."

    cd ffi

    # Need to capture the flake path before changing directories to
    # result/ffi because `result` is a nix store symlink so ../../
    # won't resolve to the ffi path containing the flake.
    FLAKE_PATH="$PWD"

    # This runs golang outside a nix shell to validate viability
    # without the env setup performed by a nix shell
    GO="nix run $FLAKE_PATH#go"

    cd result/ffi

    # - cgocheck2 is expensive but provides complete pointer checks
    # - use hash mode ethhash since the flake builds with `--features ethhash,logger`
    GOEXPERIMENT=cgocheck2 TEST_FIREWOOD_HASH_MODE=ethhash ${GO} test ./...
