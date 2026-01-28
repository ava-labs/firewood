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
#   help                 Show this help message
#
# ENVIRONMENT
#   GH_TOKEN              GitHub token for API access (required)
#   TEST                  Predefined test name, alternative to arg (optional)
#   FIREWOOD_REF          Firewood commit/tag/branch, empty = AvalancheGo's go.mod default (optional)
#   AVALANCHEGO_REF       AvalancheGo ref to test against (default: master)
#   RUNNER                GitHub Actions runner label (default: avalanche-avalanchego-runner-2ti)
#   LIBEVM_REF            libevm ref (optional)
#   TIMEOUT_MINUTES       Workflow timeout in minutes (optional)
#   DOWNLOAD_DIR          Directory for downloaded artifacts (default: ./results)
#
#   Custom mode (when no TEST/test arg specified):
#   CONFIG                VM config (default: firewood)
#   START_BLOCK           First block number (required)
#   END_BLOCK             Last block number (required)
#   BLOCK_DIR_SRC         S3 block directory, e.g., cchain-mainnet-blocks-200-ldb (required)
#   CURRENT_STATE_DIR_SRC S3 state directory, empty = genesis run (optional)
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

err() {
    if [[ "${GITHUB_ACTIONS:-}" == "true" ]]; then
        echo "::error::$1"
    else
        echo "error: $1" >&2
    fi
}

require_gh() {
    if ! command -v gh &>/dev/null; then
        err "gh CLI not found. Run from nix shell: nix develop ./ffi"; exit 1
    fi
    if [[ -z "${GH_TOKEN:-}" ]]; then
        err "GH_TOKEN is required"; exit 1
    fi
}

# Prevent command injection via malicious ref names passed to gh CLI
validate_ref() {
    local ref="$1" name="$2"
    [[ "$ref" =~ ^[a-zA-Z0-9/_.-]+$ ]] || { err "$name contains invalid characters: $ref"; exit 1; }
}

# Verify a run's inputs match what we triggered with
# Check if a workflow run's inputs match expected values.
# Returns: 0 if inputs match, 1 if mismatch
#
# NOTE: We rely on AvalancheGo's workflow job naming convention:
#   "c-chain-reexecution (start, end, block-dir, config, runner...)"
# If AvalancheGo changes this format, this matching logic will need updating.
run_inputs_match() {
    local run_id="$1" expected_test="$2" expected_config="$3" expected_start="$4" expected_end="$5" expected_runner="$6"
    
    # Get the c-chain-reexecution job name (second job, after define-matrix)
    local job_name
    job_name=$(gh run view "$run_id" --repo "$AVALANCHEGO_REPO" --json jobs \
        --jq '.jobs[] | select(.name | startswith("c-chain-reexecution")) | .name // ""' 2>/dev/null) || return 1
    
    # If job hasn't started yet (only define-matrix exists), accept for now
    [[ -z "$job_name" ]] && return 0
    
    # If we have a test name, check if it appears in job name
    if [[ -n "$expected_test" ]]; then
        [[ "$job_name" == *"$expected_test"* ]] && return 0
        return 1
    fi
    
    # For custom mode, check that our params appear in job name
    # Job name: "c-chain-reexecution (1, 100, cchain-mainnet-blocks-200-ldb, firewood, runner...)"
    local match=true
    [[ -n "$expected_start" && "$job_name" != *"$expected_start"* ]] && match=false
    [[ -n "$expected_config" && "$job_name" != *"$expected_config"* ]] && match=false
    [[ -n "$expected_runner" && "$job_name" != *"$expected_runner"* ]] && match=false
    
    $match && return 0
    
    # Fallback: if no identifiers to match, accept the run
    [[ -z "$expected_test" && -z "$expected_start" && -z "$expected_runner" ]] && return 0
    
    return 1
}

poll_workflow_registration() {
    local trigger_time="$1"
    local expected_test="${2:-}"
    local expected_config="${3:-}"
    local expected_start="${4:-}"
    local expected_end="${5:-}"
    local expected_runner="${6:-}"
    
    log "Waiting for workflow to register (looking for runs after $trigger_time)..."
    
    # Verify we can list runs (fail fast on permission issues)
    if ! gh run list --repo "$AVALANCHEGO_REPO" --workflow "$WORKFLOW_NAME" --limit 1 &>/dev/null; then
        err "Cannot list runs in $AVALANCHEGO_REPO. Check token permissions."; exit 1
    fi
    
    for ((i=0; i<POLL_TIMEOUT; i++)); do
        sleep "$POLL_INTERVAL"
        local raw_output
        raw_output=$(gh run list \
            --repo "$AVALANCHEGO_REPO" \
            --workflow "$WORKFLOW_NAME" \
            --limit 10 \
            --json databaseId,createdAt 2>&1) || {
            log "gh run list failed: $raw_output"
            continue
        }
        
        # Debug: show first result on first iteration
        ((i == 0)) && log "Latest run: $(echo "$raw_output" | jq -c '.[0] // "none"')"
        
        # Get candidate runs (created after trigger_time), oldest first
        local candidates
        candidates=$(echo "$raw_output" | jq -r --arg t "$trigger_time" \
            '[.[] | select(.createdAt > $t)] | reverse | .[].databaseId')
        
        # Check each candidate's inputs to find our run
        for run_id in $candidates; do
            if run_inputs_match "$run_id" "$expected_test" "$expected_config" "$expected_start" "$expected_end" "$expected_runner"; then
                echo "$run_id"
                return 0
            fi
        done
        
        # Progress indicator every 10 seconds
        ((i % 10 == 0)) && ((i > 0)) && log "Polling (${i}s)"
    done
    
    err "Workflow not found after ${POLL_TIMEOUT}s. The workflow may have been triggered but couldn't be detected. Check: https://github.com/$AVALANCHEGO_REPO/actions/workflows"; exit 1
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
    [[ -n "$TIMEOUT_MINUTES" ]] && args+=(-f timeout-minutes="$TIMEOUT_MINUTES")
    
    # Build with-dependencies string (format: "firewood=abc,libevm=xyz")
    local deps=""
    [[ -n "$firewood" ]] && deps="firewood=$firewood"
    [[ -n "$LIBEVM_REF" ]] && deps="${deps:+$deps,}libevm=$LIBEVM_REF"
    [[ -n "$deps" ]] && args+=(-f with-dependencies="$deps")
    
    if [[ -n "$test" ]]; then
        args+=(-f test="$test")
        log "Triggering: $test"
    else
        # Custom mode: block params required, CURRENT_STATE_DIR_SRC optional (empty = genesis)
        [[ -z "${START_BLOCK:-}${END_BLOCK:-}${BLOCK_DIR_SRC:-}" ]] && \
            err "Provide a test name or set START_BLOCK, END_BLOCK, BLOCK_DIR_SRC"; exit 1
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
    
    # Pass test/custom params to help identify our specific run among concurrent triggers
    poll_workflow_registration "$trigger_time" "$test" "$CONFIG" "${START_BLOCK:-}" "${END_BLOCK:-}" "$RUNNER"
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
    
    [[ -f "$output_file" ]] || { err "No benchmark results found"; exit 1; }
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
    [[ -z "${1:-}" ]] && { err "usage: $0 status <run_id>"; exit 1; }
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
  help             Show this help message

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
