# List available recipes
default:
    ./scripts/run-just.sh --list

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

# Check if the FFI flake is up-to-date (requires clean git tree)
check-ffi-flake: check-nix
    #!/usr/bin/env bash
    set -euo pipefail
    ./scripts/run-just.sh update-ffi-flake
    ./scripts/run-just.sh check-clean-branch

# Check if the golang version is set consistently (requires clean git tree)
check-golang-version: check-nix
    #!/usr/bin/env bash
    set -euo pipefail

    # Exit only at the end if any of the checks set FAILED=1
    FAILED=

    cd ffi

    TOOLCHAIN_VERSION=$(nix develop --command bash -c "go mod edit -json | jq -r '.Toolchain'")
    echo "toolchain version in ffi/go.mod is ${TOOLCHAIN_VERSION}"

    ETH_TESTS_VERSION=$(nix develop --command bash -c "cd tests/eth && go mod edit -json | jq -r '.Toolchain'")
    echo "toolchain version in ffi/tests/eth/go.mod is ${ETH_TESTS_VERSION}"

    if [[ "${TOOLCHAIN_VERSION}" != "${ETH_TESTS_VERSION}" ]]; then
        echo "❌ toolchain version in ffi/tests/eth/go.mod should be ${TOOLCHAIN_VERSION}"
        FAILED=1
    fi

    FIREWOOD_TESTS_VERSION=$(nix develop --command bash -c "cd tests/firewood && go mod edit -json | jq -r '.Toolchain'")
    echo "toolchain version in ffi/tests/firewood/go.mod is ${FIREWOOD_TESTS_VERSION}"

    if [[ "${TOOLCHAIN_VERSION}" != "${FIREWOOD_TESTS_VERSION}" ]]; then
        echo "❌ toolchain version in ffi/tests/firewood/go.mod should be ${TOOLCHAIN_VERSION}"
        FAILED=1
    fi

    NIX_VERSION=$(nix run .#go -- version | awk '{print $3}')
    echo "golang provided by ffi/flake.nix is ${NIX_VERSION}"

    if [[ "${TOOLCHAIN_VERSION}" != "${NIX_VERSION}" ]]; then
        echo "❌ golang provided by ffi/flake/nix should be ${TOOLCHAIN_VERSION}"
        echo "It will be necessary to update the golang.url in ffi/flake.nix to point to a SHA of"\
             "AvalancheGo whose nix/go/flake.nix provides ${TOOLCHAIN_VERSION}."
    fi

    if [[ -n "${FAILED}" ]]; then
        exit 1
    fi

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

# Adds go workspace for user experience consistency
setup-go-workspace:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ -f "go.work" ]; then
        rm go.work go.work.sum
    fi
    go work init ./ffi ./ffi/tests/eth ./ffi/tests/firewood

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

    echo "running ffi tests against bindings built by nix..."

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

# Ensure the FFI flake is up-to-date
update-ffi-flake: check-nix
    #!/usr/bin/env bash
    set -euo pipefail
    cd ffi

    echo "ensuring flake lock file is current for golang"
    nix flake update golang

    echo "checking for a consistent golang verion"
    ../scripts/run-just.sh check-golang-version

# Trigger task based reexecution in AvalancheGo
trigger-reexecution firewood avalanchego task runner libevm="" timeout-minutes="120":
    just _trigger-workflow "{{ firewood }}" "{{ libevm }}" "{{ avalanchego }}" "{{ runner }}" "{{ timeout-minutes }}" \
        -f "task={{ task }}"

# Trigger custom param reexecution in AvalancheGo
trigger-custom-reexecution firewood avalanchego config start-block end-block block-dir-src current-state-dir-src runner libevm="" timeout-minutes="120":
    just _trigger-workflow "{{ firewood }}" "{{ libevm }}" "{{ avalanchego }}" "{{ runner }}" "{{ timeout-minutes }}" \
        -f "config={{ config }}" \
        -f "start-block={{ start-block }}" \
        -f "end-block={{ end-block }}" \
        -f "block-dir-src={{ block-dir-src }}" \
        -f "current-state-dir-src={{ current-state-dir-src }}"

# Wait for reexecution run to complete
wait-reexecution run_id: check-nix
    nix run ./ffi#gh -- run watch "{{ run_id }}" --repo ava-labs/avalanchego --exit-status

# Download reexecution results
download-reexecution-results run_id: check-nix
    #!/usr/bin/env -S bash -euo pipefail
    mkdir -p ./results
    nix run ./ffi#gh -- run download "{{ run_id }}" \
        --repo ava-labs/avalanchego \
        --name benchmark-output \
        --dir ./results
    cat ./results/benchmark-output.txt

# List recent reexecution runs in AvalancheGo
list-reexecutions: check-nix
    nix run ./ffi#gh -- run list --repo ava-labs/avalanchego --workflow="C-Chain Re-Execution Benchmark w/ Container" --limit 10

# Internal helpers (not intended for direct use)

_trigger-workflow firewood libevm avalanchego runner timeout-minutes +extra_flags: check-nix
    #!/usr/bin/env -S bash -euo pipefail
    GH="nix run ./ffi#gh --"
    
    # Validate refs contain only safe characters
    if [[ ! "{{ firewood }}" =~ ^[a-zA-Z0-9/_.-]+$ ]]; then
        echo "Error: firewood ref contains invalid characters: {{ firewood }}" >&2
        exit 1
    fi
    if [[ -n "{{ libevm }}" && ! "{{ libevm }}" =~ ^[a-zA-Z0-9/_.-]+$ ]]; then
        echo "Error: libevm ref contains invalid characters: {{ libevm }}" >&2
        exit 1
    fi
    
    WITH_VALUE="firewood={{ firewood }}"
    if [ -n "{{ libevm }}" ]; then
        WITH_VALUE="${WITH_VALUE},libevm={{ libevm }}"
    fi
    
    $GH workflow run "C-Chain Re-Execution Benchmark w/ Container" \
        --repo ava-labs/avalanchego \
        --ref "{{ avalanchego }}" \
        -f runner="{{ runner }}" \
        -f timeout-minutes="{{ timeout-minutes }}" \
        -f with="$WITH_VALUE" \
        {{ extra_flags }}
    
    just _wait-for-workflow-run

_wait-for-workflow-run: check-nix
    #!/usr/bin/env -S bash -euo pipefail
    GH="nix run ./ffi#gh --"
    for i in {1..18}; do
        sleep 10
        RUN_ID=$($GH run list \
            --repo ava-labs/avalanchego \
            --workflow "C-Chain Re-Execution Benchmark w/ Container" \
            --limit 5 \
            --json databaseId,createdAt \
            --jq '[.[] | select(.createdAt | fromdateiso8601 > (now - 180))] | .[0].databaseId')
        if [ -n "$RUN_ID" ] && [ "$RUN_ID" != "null" ]; then
            echo "$RUN_ID"
            exit 0
        fi
    done
    echo "Error: Could not find workflow run after 3min. Check AvalancheGo logs." >&2
    exit 1
