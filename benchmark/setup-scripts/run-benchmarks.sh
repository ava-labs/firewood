#!/usr/bin/env bash

DB="$HOME/firewood/benchmark-db"

WITH_LOGGING=${WITH_LOGGING:-false}

declare -a args=(
    --stats-dump
)

if [[ "$WITH_LOGGING" == "true" ]]; then
    args+=(--log-level debug)
fi

targetdir=$(mktemp -d -t firewood-benchmark-XXXXXX)
trap 'rm -rf "$targetdir"' EXIT

# build in an isolated target directory so we don't pollute the workspace or
# worry about the benchmark binary disappearing underneath us
cargo build --target-dir "$targetdir" --profile maxperf --package firewood-benchmark --bin benchmark

declare -r cmd="$targetdir/maxperf/benchmark"

function @run-test() {
    local test_name="$1"
    shift
    echo "Running test: $test_name"
    nohup time "$cmd" "${args[@]}" "$@" create &
    wait
    nohup time "$cmd" "${args[@]}" "$@" zipf &
    wait
    nohup time "$cmd" "${args[@]}" "$@" single &
    wait
    nohup time "$cmd" "${args[@]}" "$@" ten-k-random &
    wait
}

# 10M rows:
@run-test "10M rows" -n 1000 -d "$DB-10M"

# 50M rows:
@run-test "50M rows" -n 5000 -d "$DB-50M"

# 100M rows:
@run-test "100M rows" -n 10000 -d "$DB-100M"

# 500M rows:
@run-test "500M rows" -n 50000 -d "$DB-500M"

# 1B rows:
@run-test "1B rows" -n 100000 -d "$DB-1B"
