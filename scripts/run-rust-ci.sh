#!/usr/bin/env bash

set -euo pipefail

# This is the single source of truth for the Cargo arguments behind the full CI
# matrix. Every profile here is portable to macOS, so GitHub Actions and the
# local Just aggregators pass the same set. Checks that genuinely require Linux
# (differential fuzzing) live in the workflows, not here.

usage() {
    cat <<'EOF'
Usage:
  scripts/run-rust-ci.sh help
  scripts/run-rust-ci.sh <command> <profile>
  scripts/run-rust-ci.sh bench <profile> <bench-target> [harness-arg...]

Commands:
  help               Show this usage information
  check              Run cargo check with the CI profile
  build              Run cargo build with the CI profile
  build-benches      Build every workspace bench target with the CI profile
  bench              Run one bench target by name with the CI profile; trailing
                     arguments are passed to the Criterion harness (e.g.
                     --output-format bencher)
  clippy             Run the pinned PR clippy toolchain with the CI profile
  clippy-nightly     Run the latest nightly clippy toolchain with the CI profile
  test               Run cargo-nextest with the CI profile
  benchmark-example  Run the benchmark example exercised by CI
  insert-example     Run the insert example exercised by CI

Profiles:
  debug-no-default-features
  debug-no-features (default features)
  debug-ethhash-logger
  debug-all-features (io-uring is active on Linux only; needs rustc 1.94.1+,
                      above the 1.94.0 workspace MSRV, because fwdctl's
                      launch feature pulls in the AWS SDK)
  maxperf-ethhash-logger
  maxperf-ethhash-logger-test_utils (benchmarks; test_utils gates the internals
                      several bench targets declare in required-features)

Use scripts/list-bench-targets.sh to enumerate valid bench target names.
EOF
}

if [[ $# -eq 1 && $1 == help ]]; then
    usage
    exit 0
fi

if [[ $# -lt 2 ]]; then
    usage >&2
    exit 2
fi

command=$1
profile=$2
shift 2

# Only `bench` takes trailing arguments. Rejecting them elsewhere turns a typo
# in a workflow or Just recipe into a failure instead of a silently ignored
# argument.
if [[ $command != bench && $# -ne 0 ]]; then
    echo "error: Rust CI command '$command' does not accept extra arguments: $*" >&2
    usage >&2
    exit 2
fi

cargo_args=()
nextest_args=()
case "$profile" in
    debug-no-default-features)
        cargo_args=(--no-default-features)
        nextest_args=(--no-default-features)
        ;;
    debug-no-features)
        ;;
    debug-ethhash-logger)
        cargo_args=(--features ethhash,logger)
        nextest_args=(--features ethhash,logger)
        ;;
    debug-all-features)
        cargo_args=(--all-features)
        nextest_args=(--all-features)
        ;;
    maxperf-ethhash-logger)
        cargo_args=(--profile maxperf --features ethhash,logger)
        nextest_args=(--cargo-profile maxperf --features ethhash,logger)
        ;;
    maxperf-ethhash-logger-test_utils)
        cargo_args=(--profile maxperf --features ethhash,logger,test_utils)
        nextest_args=(--cargo-profile maxperf --features ethhash,logger,test_utils)
        ;;
    *)
        echo "error: unknown Rust CI profile '$profile'" >&2
        usage >&2
        exit 2
        ;;
esac

# Expanding an empty array with "${arr[@]}" is an unbound-variable error
# under `set -u` on bash < 4.4 (macOS ships 3.2), so expansion sites use
# ${arr[@]+"${arr[@]}"} instead.
case "$command" in
    check)
        cargo check --frozen ${cargo_args[@]+"${cargo_args[@]}"} --workspace --all-targets
        ;;
    build)
        cargo build --frozen ${cargo_args[@]+"${cargo_args[@]}"} --workspace --all-targets
        ;;
    build-benches)
        cargo build --frozen ${cargo_args[@]+"${cargo_args[@]}"} --workspace --benches
        ;;
    bench)
        if [[ $# -lt 1 ]]; then
            echo "error: 'bench' requires a bench target name" >&2
            usage >&2
            exit 2
        fi
        bench_target=$1
        shift
        # `--workspace`, not `-p <package>`. Cargo resolves dependency features
        # from the package selection, so narrowing to one package resolves a
        # different feature set than `build-benches` does and recompiles behind
        # an otherwise warm target directory. Workspace scoping also lets one
        # feature string cover every member: a feature applies to whichever
        # members declare it and is a no-op for the rest, whereas
        # `-p firewood-triehash --features ethhash,...` is an error because that
        # package declares no features at all.
        #
        # Selecting a single target by name requires bench names to be unique
        # across the workspace; scripts/list-bench-targets.sh asserts that.
        # Unlike the plural `--benches`, singular `--bench` fails loudly when a
        # target's required-features are not enabled instead of skipping it.
        cargo bench --frozen ${cargo_args[@]+"${cargo_args[@]}"} --workspace --bench "$bench_target" -- --noplot ${@+"$@"}
        ;;
    clippy)
        cargo +nightly-2026-07-05 clippy --locked ${cargo_args[@]+"${cargo_args[@]}"} --workspace --all-targets -- -D warnings
        ;;
    clippy-nightly)
        cargo +nightly clippy --locked ${cargo_args[@]+"${cargo_args[@]}"} --workspace --all-targets -- -D warnings
        ;;
    test)
        cargo nextest run --locked --profile ci --verbose ${nextest_args[@]+"${nextest_args[@]}"}
        ;;
    benchmark-example)
        cargo run --locked ${cargo_args[@]+"${cargo_args[@]}"} --bin benchmark -- --number-of-batches 100 --batch-size 1000 create
        ;;
    insert-example)
        cargo run --locked ${cargo_args[@]+"${cargo_args[@]}"} --example insert
        ;;
    *)
        echo "error: unknown Rust CI command '$command'" >&2
        usage >&2
        exit 2
        ;;
esac
