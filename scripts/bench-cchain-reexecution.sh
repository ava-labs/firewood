#!/usr/bin/env bash
set -euo pipefail

# Triggers and monitors C-Chain re-execution benchmarks in AvalancheGo.
#
# USAGE
#   ./bench-cchain-reexecution.sh <command> [args]
#
# COMMANDS
#   trigger [test]       Trigger benchmark, wait, download results
#   status <run_id>      Check run status
#   list                 List recent runs
#   tests                Show available tests
#   help                 Show this help
#
# ENVIRONMENT
#   GH_TOKEN              (string, required)                          GitHub token for API access
#   TEST                  (string, optional)                          Predefined test name (alternative to arg)
#   FIREWOOD_REF          (string, optional)                          Firewood commit/tag/branch (empty = AvalancheGo's go.mod default)
#   AVALANCHEGO_REF       (string, master)                            AvalancheGo ref to test against
#   RUNNER                (string, avalanche-avalanchego-runner-2ti)  GitHub Actions runner label
#   LIBEVM_REF            (string, optional)                          Optional libevm ref
#   TIMEOUT_MINUTES       (int, optional)                             Workflow timeout
#   DOWNLOAD_DIR          (string, ./results)                         Directory for downloaded artifacts
#
#   Custom mode (when no TEST/test arg specified):
#   CONFIG                (string, firewood)  VM config (https://github.com/ava-labs/avalanchego/blob/3c645de551294b8db0f695563e386a2f38c1aded/tests/reexecute/c/vm_reexecute.go#L64)
#   START_BLOCK           (required)  First block number
#   END_BLOCK             (required)  Last block number
#   BLOCK_DIR_SRC         (required)  S3 block directory (without S3:// prefix, e.g., cchain-mainnet-blocks-200-ldb)
#   CURRENT_STATE_DIR_SRC (optional)  S3 state directory (empty = genesis run)
#
#
# EXAMPLES
#   ./bench-cchain-reexecution.sh trigger firewood-101-250k
#
#   TEST=firewood-33m-40m FIREWOOD_REF=v0.1.0 ./bench-cchain-reexecution.sh trigger
#
#   START_BLOCK=101 END_BLOCK=250000 \
#     BLOCK_DIR_SRC=cchain-mainnet-blocks-1m-ldb \
#     CURRENT_STATE_DIR_SRC=cchain-current-state-mainnet-ldb \
#     ./bench-cchain-reexecution.sh trigger
#
#   ./bench-cchain-reexecution.sh status 12345678

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

AVALANCHEGO_REPO="ava-labs/avalanchego"
WORKFLOW_NAME="C-Chain Re-Execution Benchmark w/ Container"

: "${FIREWOOD_REF:=}"
: "${AVALANCHEGO_REF:=master}"
: "${RUNNER:=avalanche-avalanchego-runner-2ti}"
: "${LIBEVM_REF:=}"
: "${TIMEOUT_MINUTES:=}"
: "${CONFIG:=firewood}"
: "${DOWNLOAD_DIR:=${REPO_ROOT}/results}"

# Polling config for workflow registration. gh workflow run doesn't return
# the run ID, so we poll until the run appears. 180s accommodates busy runners.
POLL_INTERVAL=1
POLL_TIMEOUT=180

# Subset of tests for discoverability. AvalancheGo is the source of truth:
# https://github.com/ava-labs/avalanchego/blob/master/scripts/benchmark_cchain_range.sh
declare -A TESTS=(
    ["firewood-101-250k"]="Blocks 101-250k"
    ["firewood-archive-101-250k"]="Blocks 101-250k (archive)"
    ["firewood-33m-33m500k"]="Blocks 33m-33.5m"
    ["firewood-archive-33m-33m500k"]="Blocks 33m-33.5m (archive)"
    ["firewood-33m-40m"]="Blocks 33m-40m"
    ["firewood-archive-33m-40m"]="Blocks 33m-40m (archive)"
)

log() { echo "==> $1" >&2; }

# Use GitHub Actions error annotation format in CI for better visibility
err() {
    if [[ "${GITHUB_ACTIONS:-}" == "true" ]]; then
        echo "::error::$1"
    else
        echo "error: $1" >&2
    fi
}

die() { err "$1"; exit 1; }

require_gh() {
    if ! command -v gh &>/dev/null; then
        die "gh CLI not found. Run from nix shell: nix develop ./ffi"
    fi
    if [[ -z "${GH_TOKEN:-}" ]]; then
        die "GH_TOKEN is required"
    fi
}

# Prevent command injection via malicious ref names passed to gh CLI
validate_ref() {
    local ref="$1" name="$2"
    [[ "$ref" =~ ^[a-zA-Z0-9/_.-]+$ ]] || die "$name contains invalid characters: $ref"
}

poll_workflow_registration() {
    local trigger_time="$1"
    log "Waiting for workflow to register (looking for runs after $trigger_time)..."
    
    # Verify we can list runs (fail fast on permission issues)
    if ! gh run list --repo "$AVALANCHEGO_REPO" --workflow "$WORKFLOW_NAME" --limit 1 &>/dev/null; then
        die "Cannot list runs in $AVALANCHEGO_REPO. Check token permissions."
    fi
    
    for ((i=0; i<POLL_TIMEOUT; i++)); do
        sleep "$POLL_INTERVAL"
        local run_id raw_output
        raw_output=$(gh run list \
            --repo "$AVALANCHEGO_REPO" \
            --workflow "$WORKFLOW_NAME" \
            --limit 5 \
            --json databaseId,createdAt 2>&1) || {
            log "gh run list failed: $raw_output"
            continue
        }
        
        # Debug: show first result on first iteration
        ((i == 0)) && log "Latest run: $(echo "$raw_output" | jq -c '.[0] // "none"')"
        
        run_id=$(echo "$raw_output" | jq -r --arg t "$trigger_time" '[.[] | select(.createdAt > $t)] | .[0].databaseId // empty')
        
        if [[ -n "$run_id" && "$run_id" != "null" ]]; then
            echo "$run_id"
            return 0
        fi
        
        # Progress indicator every 10 seconds
        ((i % 10 == 0)) && ((i > 0)) && log "Polling (${i}s)"
    done
    
    die "Workflow not found after ${POLL_TIMEOUT}s. The workflow may have been triggered but couldn't be detected. Check: https://github.com/$AVALANCHEGO_REPO/actions/workflows"
}

trigger_workflow() {
    local test="${1:-}"
    local firewood="$FIREWOOD_REF"
    
    # Validate refs if provided
    if [[ -n "$firewood" ]]; then
        # Resolve HEAD to commit SHA for reproducibility
        [[ "$firewood" == "HEAD" ]] && firewood=$(git -C "$REPO_ROOT" rev-parse HEAD)
        validate_ref "$firewood" "FIREWOOD_REF"
    fi
    [[ -n "$LIBEVM_REF" ]] && validate_ref "$LIBEVM_REF" "LIBEVM_REF"
    
    local args=(-f runner="$RUNNER")
    [[ -n "$firewood" ]] && args+=(-f firewood-ref="$firewood")
    [[ -n "$LIBEVM_REF" ]] && args+=(-f libevm-ref="$LIBEVM_REF")
    [[ -n "$TIMEOUT_MINUTES" ]] && args+=(-f timeout-minutes="$TIMEOUT_MINUTES")
    
    if [[ -n "$test" ]]; then
        args+=(-f test="$test")
        log "Triggering: $test"
    else
        # Custom mode: block params required, CURRENT_STATE_DIR_SRC optional (empty = genesis)
        [[ -z "${START_BLOCK:-}${END_BLOCK:-}${BLOCK_DIR_SRC:-}" ]] && \
            die "Provide a test name or set START_BLOCK, END_BLOCK, BLOCK_DIR_SRC"
        : "${START_BLOCK:?START_BLOCK required}"
        : "${END_BLOCK:?END_BLOCK required}"
        : "${BLOCK_DIR_SRC:?BLOCK_DIR_SRC required}"
        args+=(
            -f config="$CONFIG"
            -f start-block="$START_BLOCK"
            -f end-block="$END_BLOCK"
            -f block-dir-src="$BLOCK_DIR_SRC"
        )
        [[ -n "${CURRENT_STATE_DIR_SRC:-}" ]] && args+=(-f current-state-dir-src="$CURRENT_STATE_DIR_SRC")
        log "Triggering: $CONFIG $START_BLOCK-$END_BLOCK${CURRENT_STATE_DIR_SRC:+ (with state)}"
    fi
    
    log "avalanchego: $AVALANCHEGO_REF"
    log "firewood:    ${firewood:-<AvalancheGo go.mod default>}"
    log "runner:      $RUNNER"
    
    # Record time BEFORE triggering to avoid race condition: we only look for
    # runs created after this timestamp, so concurrent triggers don't collide
    local trigger_time
    trigger_time=$(date -u +%Y-%m-%dT%H:%M:%SZ)
    
    gh workflow run "$WORKFLOW_NAME" \
        --repo "$AVALANCHEGO_REPO" \
        --ref "$AVALANCHEGO_REF" \
        "${args[@]}"
    
    poll_workflow_registration "$trigger_time"
}

wait_for_completion() {
    local run_id="$1"
    log "Waiting for run $run_id"
    log "https://github.com/${AVALANCHEGO_REPO}/actions/runs/${run_id}"
    gh run watch "$run_id" --repo "$AVALANCHEGO_REPO" --exit-status
}

download_artifact() {
    local run_id="$1"
    local output_file="$DOWNLOAD_DIR/benchmark-output.json"
    
    log "Downloading artifact..."
    mkdir -p "$DOWNLOAD_DIR"
    gh run download "$run_id" \
        --repo "$AVALANCHEGO_REPO" \
        --pattern "benchmark-output-*" \
        --dir "$DOWNLOAD_DIR"
    
    # Flatten: gh extracts to $DOWNLOAD_DIR/<artifact-name>/benchmark-output.json
    find "$DOWNLOAD_DIR" -name "benchmark-output.json" -type f -exec mv {} "$output_file" \;
    find "$DOWNLOAD_DIR" -mindepth 1 -type d -delete 2>/dev/null || true
    
    [[ -f "$output_file" ]] || die "No benchmark results found"
    log "Results: $output_file"
}

check_status() {
    local run_id="$1"
    echo "https://github.com/${AVALANCHEGO_REPO}/actions/runs/${run_id}"
    
    local status
    status=$(gh run view "$run_id" --repo "$AVALANCHEGO_REPO" --json status,conclusion \
        --jq '.status + " (" + (.conclusion // "in progress") + ")"')
    echo "status: $status"
}

list_runs() {
    gh run list --repo "$AVALANCHEGO_REPO" --workflow "$WORKFLOW_NAME" --limit 10
}

list_tests() {
    for test in "${!TESTS[@]}"; do
        [[ "$test" == firewood* ]] && printf "  %-25s %s\n" "$test" "${TESTS[$test]}"
    done | sort
}

cmd_trigger() {
    require_gh
    
    local test="${1:-${TEST:-}}"
    local run_id
    run_id=$(trigger_workflow "$test")
    log "Run ID: $run_id"
    
    wait_for_completion "$run_id"
    download_artifact "$run_id"
    log "Done"
}

cmd_status() {
    [[ -z "${1:-}" ]] && die "usage: $0 status <run_id>"
    require_gh
    check_status "$1"
}

cmd_list() {
    require_gh
    list_runs
}

cmd_tests() {
    list_tests
}

cmd_help() {
    cat <<EOF
USAGE
  ./bench-cchain-reexecution.sh <command> [args]

COMMANDS
  trigger [test]   Trigger benchmark, wait, download results
  status <run_id>  Check run status
  list             List recent runs
  tests            Show available tests
  help             Show this help

TESTS
EOF
    list_tests
}

main() {
    local cmd="${1:-help}"
    shift || true
    
    case "$cmd" in
        trigger)        cmd_trigger "$@" ;;
        status)         cmd_status "$@" ;;
        list)           cmd_list "$@" ;;
        tests)          cmd_tests "$@" ;;
        help|-h|--help) cmd_help ;;
        *)              err "unknown command: $cmd"; cmd_help; exit 1 ;;
    esac
}

main "$@"
