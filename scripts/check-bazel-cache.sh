#!/usr/bin/env bash

set -euo pipefail

# Keep downloads and build outputs outside the developer's existing caches.
temporary_dir="$(mktemp -d)"
export FIREWOOD_BAZELRC="$temporary_dir/bazelrc"
export FIREWOOD_BAZEL_OUTPUT_BASE="$temporary_dir/validation-output-base"
export BAZELISK_HOME="$temporary_dir/bazelisk"
export REPOSITORY_CACHE_DIR="$temporary_dir/repository-cache"
export GO_MOD_CACHE_DIR="$temporary_dir/go-mod-cache"

cleanup() {
    local status=$?
    ./scripts/run-bazel.sh shutdown > /dev/null 2>&1 || true
    # Go module sources and Bazel outputs can contain read-only directories.
    find "$temporary_dir" -type d -exec chmod u+w {} + || true
    rm -rf "$temporary_dir" || true
    return "$status"
}
trap cleanup EXIT

mkdir -p "$BAZELISK_HOME" "$REPOSITORY_CACHE_DIR" "$GO_MOD_CACHE_DIR"
# Use the CI configuration helper without writing into the developer's home.
HOME="$temporary_dir" BAZEL_REMOTE_CACHE_URL='' BAZEL_REMOTE_CACHE_AUTH_HEADER='' \
    ./.github/actions/setup-bazel/configure-bazelrc.sh
mv "$temporary_dir/.bazelrc" "$FIREWOOD_BAZELRC"
# Do not replace the developer's workspace convenience symlinks.
printf '%s\n' 'build --symlink_prefix=/' >> "$FIREWOOD_BAZELRC"

# The preparation script overrides the output base and deletes it on exit.
echo "Preparing Bazel inputs in empty caches..."
./.github/actions/setup-bazel/cache-ci-dependencies.sh

printf '%s\n' \
    'common --repository_disable_download' \
    'common --repo_env=GOPROXY=off' >> "$FIREWOOD_BAZELRC"

echo "Checking metadata and building from prepared inputs in a fresh output base..."
./scripts/run-just.sh ci-check-bazel-metadata
./scripts/run-just.sh ci-check-bazel-toolchain
./scripts/run-just.sh ci-bazel-ffi
echo "Bazel input-cache validation passed."
