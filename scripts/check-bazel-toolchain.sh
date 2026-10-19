#!/usr/bin/env bash

set -euo pipefail

cargo_edition=$(awk -F '"' '/^edition[[:space:]]*=/ { print $2; exit }' Cargo.toml)
cargo_version=$(awk -F '"' '/^rust-version[[:space:]]*=/ { print $2; exit }' Cargo.toml)
bazel_toolchain=$(sed -n '/rust\.toolchain(/,/^)/p' MODULE.bazel)
bazel_edition=$(awk -F '"' '/edition[[:space:]]*=/ { print $2; exit }' <<<"$bazel_toolchain")
mapfile -t bazel_versions < <(awk -F '"' '/versions[[:space:]]*=/ { for (i = 2; i <= NF; i += 2) print $i }' <<<"$bazel_toolchain")

if [[ -z "$cargo_edition" || -z "$cargo_version" || -z "$bazel_edition" || ${#bazel_versions[@]} -eq 0 ]]; then
    echo "error: could not read the Cargo or Bazel Rust toolchain" >&2
    exit 1
fi

if [[ "$cargo_edition" != "$bazel_edition" ]]; then
    echo "error: Cargo edition is $cargo_edition; Bazel edition is $bazel_edition" >&2
    exit 1
fi

for version in "${bazel_versions[@]}"; do
    if [[ "$cargo_version" == "$version" ]]; then
        exit 0
    fi
done

echo "error: Cargo rust-version is $cargo_version; Bazel versions are ${bazel_versions[*]}" >&2
exit 1
