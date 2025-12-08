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

# Trigger Reexecution Benchmark
# Usage: just trigger-benchmark <firewood> <avalanchego> <task> <runner> [libevm]
trigger-benchmark firewood avalanchego task runner libevm="": check-nix
    #!/usr/bin/env -S bash -euo pipefail
    
    GH="nix run ./ffi#gh --"
    
    LIBEVM_FLAG=""
    if [ -n "{{ libevm }}" ]; then
        LIBEVM_FLAG="-f libevm={{ libevm }}"
    fi
    
    $GH workflow run "Firewood Reexecution Benchmark" \
        --repo ava-labs/avalanchego \
        --ref "{{ avalanchego }}" \
        -f firewood="{{ firewood }}" \
        -f task="{{ task }}" \
        -f runner="{{ runner }}" \
        $LIBEVM_FLAG
    
    # Wait for GitHub to register the run (retry up to 3min)
    for i in {1..18}; do
        sleep 10
        RUN_ID=$($GH run list \
            --repo ava-labs/avalanchego \
            --workflow "Firewood Reexecution Benchmark" \
            --limit 5 \
            --json databaseId,createdAt \
            --jq '[.[] | select(.createdAt | fromdateiso8601 > (now - 180))] | .[0].databaseId')
        
        if [ -n "$RUN_ID" ] && [ "$RUN_ID" != "null" ]; then
            echo "$RUN_ID"
            exit 0
        fi
    done
    
    echo "Error: Could not find triggered workflow run after 3min" >&2
    exit 1

# Wait for reexecution benchmark run to complete (2h timeout)
# Usage: just wait-benchmark <run_id>
wait-benchmark run_id: check-nix
    #!/usr/bin/env -S bash -euo pipefail
    timeout -k 1m 2h nix run ./ffi#gh -- run watch "{{ run_id }}" --repo ava-labs/avalanchego --exit-status

# Download benchmark results
# Usage: just download-benchmark-results <run_id>
download-benchmark-results run_id: check-nix
    #!/usr/bin/env -S bash -euo pipefail
    mkdir -p ./results
    nix run ./ffi#gh -- run download "{{ run_id }}" \
        --repo ava-labs/avalanchego \
        --name benchmark-output \
        --dir ./results
    cat ./results/benchmark-output.txt

# Run full benchmark: trigger, wait, download (composes the above)
# Usage: just benchmark [firewood] [avalanchego] [task] [runner] [libevm]
benchmark firewood="HEAD" avalanchego="master" task="c-chain-reexecution-firewood-101-250k" runner="avalanche-avalanchego-runner-2ti" libevm="": check-nix
    #!/usr/bin/env -S bash -euo pipefail
    
    FIREWOOD="{{ firewood }}"
    if [[ "$FIREWOOD" == "HEAD" ]]; then
        FIREWOOD=$(git rev-parse HEAD)
    fi
    
    echo "Triggering reexecution benchmark in AvalancheGo with parameters:"
    echo "  Firewood: $FIREWOOD"
    echo "  AvalancheGo: {{ avalanchego }}"
    echo "  Task: {{ task }}"
    echo "  Runner: {{ runner }}"
    if [ -n "{{ libevm }}" ]; then
        echo "  LibEVM: {{ libevm }}"
    else
        echo "  LibEVM: (not set)"
    else
        echo "LibEVM: default"
    fi
    echo ""
    
    RUN_ID=$(just trigger-benchmark "$FIREWOOD" "{{ avalanchego }}" "{{ task }}" "{{ runner }}" "{{ libevm }}")
    echo "  Run ID: $RUN_ID"
    echo "  Monitor progress here: https://github.com/ava-labs/avalanchego/actions/runs/$RUN_ID"
    echo ""
    
    echo "Waiting for benchmark completion..."
    just wait-benchmark "$RUN_ID"
    echo ""
    
    echo "Downloading results..."
    just download-benchmark-results "$RUN_ID"
    echo ""
    echo "Results saved to: ./results/benchmark-output.txt"

# List recent AvalancheGo benchmark runs
list-benchmarks: check-nix
    #!/usr/bin/env -S bash -euo pipefail
    nix run ./ffi#gh -- run list --repo ava-labs/avalanchego --workflow="Firewood Reexecution Benchmark" --limit 10
