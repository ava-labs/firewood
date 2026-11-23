#!/usr/bin/env bash

set -euo pipefail

# Emits the workspace's Criterion bench targets as a one-line JSON array, in the
# shape GitHub Actions expects for `strategy.matrix.include`. This replaces a
# hand-maintained list of `cargo bench` invocations, so a new `[[bench]]` target
# is picked up without touching the workflow.
#
# Requires cargo and jq. Nothing else: because `--no-deps` skips dependency
# resolution, this reads only the workspace manifests, so it runs with an empty
# CARGO_HOME, with no registry, with no network, and without a prior
# `cargo fetch`. It does not even need Cargo.lock to exist. `--frozen` is
# therefore belt-and-braces rather than a check that anything is up to date;
# it is here so that this stays offline if the invocation ever grows a form
# that does resolve dependencies.
#
# Bench target names must be unique across the workspace, because the runner
# selects a single target with `--workspace --bench <name>` (see the `bench`
# command in run-rust-ci.sh for why it is not `-p <package>`). The uniqueness
# assertion below is what makes that safe.
#
# The output is a single line: a multi-line value cannot be written to
# $GITHUB_OUTPUT with a plain `echo ... >>`. Every value is a scalar, because
# `${{ matrix.<key> }}` does not usefully stringify arrays or objects.

usage() {
    cat <<'EOF'
Usage:
  scripts/list-bench-targets.sh help
  scripts/list-bench-targets.sh [filter]

Emits a one-line JSON array of {"package", "bench"} objects for every bench
target in the workspace, sorted by package then bench.

Arguments:
  filter   Optional regular expression matched against bench target names.
           Matching nothing is an error, so a typo in a workflow input fails
           instead of silently running no benchmarks.

Examples:
  scripts/list-bench-targets.sh
  scripts/list-bench-targets.sh '^proofs$'
  scripts/list-bench-targets.sh '^(rlp|triehash)$'
EOF
}

if [[ $# -eq 1 && $1 == help ]]; then
    usage
    exit 0
fi

if [[ $# -gt 1 ]]; then
    usage >&2
    exit 2
fi

filter=${1-}

cargo metadata --format-version 1 --frozen --no-deps | jq -c --arg filter "$filter" '
    # `.kind` is an array, so `.kind == "bench"` would silently match nothing.
    [ .packages[] | .name as $package | .targets[]
      | select(.kind | index("bench"))
      | { package: $package, bench: .name }
    ]
    | sort_by(.package, .bench)
    # Assert uniqueness before filtering: the runner resolves a bench name
    # against the whole workspace regardless of which names the filter kept.
    | [ .[].bench ] as $names
    | if ($names | length) != ($names | unique | length) then
          "error: bench target names must be unique across the workspace; duplicates: "
          + ($names | group_by(.) | map(select(length > 1) | .[0]) | join(", "))
          + "\n" | halt_error(1)
      else . end
    | if $filter == "" then . else map(select(.bench | test($filter))) end
    | if length == 0 then
          "error: no bench target matched filter \"\($filter)\"\n" | halt_error(1)
      else . end
'
