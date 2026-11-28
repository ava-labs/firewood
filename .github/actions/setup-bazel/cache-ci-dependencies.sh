#!/usr/bin/env bash

set -euo pipefail

# Keep this list aligned with the Bazel CI job. `bazel mod tidy` loads both
# Buildozer's module and the platform-specific generated binary repository.
# This output base must not be reused: CI persists dependency inputs only.
temporary_dir="$(mktemp -d)"
output_base="${temporary_dir}/output-base"

cleanup() {
    local status=$?
    ./scripts/run-bazel.sh --output_base="$output_base" shutdown > /dev/null 2>&1 || true
    rm -rf "$temporary_dir" || true
    return "$status"
}
trap cleanup EXIT

./scripts/run-bazel.sh --output_base="$output_base" fetch \
    //ffi:ffi_test \
    '@@buildozer+//:buildozer' \
    '@@buildozer++buildozer_binary+buildozer_binary//:buildozer.exe'
