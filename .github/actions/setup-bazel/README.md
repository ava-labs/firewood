# Bazel CI setup

Bazel CI prepares dependencies before disabling dependency downloads for builds
and tests. Missing cached inputs then fail validation instead of causing
unexpected downloads. Input caches avoid downloads; the remote cache avoids
repeated compilation and test execution.

## Table of contents

- [Usage](#usage)
- [Input-cache lifecycle](#input-cache-lifecycle)
- [Rust toolchain checksums](#rust-toolchain-checksums)
- [Remote cache](#remote-cache)
- [Changing the setup safely](#changing-the-setup-safely)
- [Shared background](#shared-background)

## Usage

[PR builds](../../workflows/ci.yaml) use remote caching when credentials are
available. The [scheduled job](../../workflows/scheduled-bazel-ffi.yaml) disables
remote caching to check that compilation and tests succeed when executed, not
just when their results are retrieved. See [action.yml](action.yml) for setup and
cache configuration.

See [Bazel source builds](../../../ffi/README.md#bazel-source-builds) for local
commands and consumer integration.

## Input-cache lifecycle

GitHub Actions saves dependency inputs, not Bazel output bases or build outputs.
Exact cache hits skip preparation; otherwise,
[cache-ci-dependencies.sh](cache-ci-dependencies.sh) fetches missing inputs.
Validation uses a separate Bazel work directory (an output base), because expanded
repositories in the preparation base can hide missing cached inputs.

Before saving a new cache entry, the action checks metadata with dependency
downloads disabled. FFI builds and tests use the same restrictions. These do not
block all network access; see AvalancheGo's
[download restrictions](https://github.com/ava-labs/avalanchego/blob/master/docs/go-toolchains-and-caches.md#download-restrictions).

If an exact-match input cache is incomplete, rerunning CI restores it without
preparing inputs again. Correct any preparation omission, then delete the
affected GitHub Actions cache entry or change its key so preparation runs again.
A change already included in the key is sufficient; see [action.yml](action.yml).
This recovery applies to input caches, not remote build results.

## Rust toolchain checksums

Cached compiler archives alone are not enough: `rules_rust` can still download a
release manifest to discover their hashes. Explicit hashes in `MODULE.bazel`
remove that lookup so toolchain setup can use the prepared archives with
dependency downloads disabled.

Do not edit the hashes by hand. Use the
[metadata update and check commands](../../../ffi/README.md#bazel-source-builds).
The [generator](../../../scripts/update-bazel-rust-toolchain-shas.sh) currently
covers Linux ARM64 and x86_64. When adding another CI platform, extend its archive
list and validate cache preparation on that platform.

## Remote cache

The PR workflow passes these optional repository secrets to the action:

- `BAZEL_REMOTE_CACHE_URL`: a cache endpoint using `grpcs://`.
- `BAZEL_REMOTE_CACHE_AUTH_HEADER`: the complete header expected by the server,
  for example `Authorization=Bearer <token>`.

If either value is missing, setup warns and leaves remote caching disabled.
With both present, [configure-bazelrc.sh](configure-bazelrc.sh) enables reads and
uploads. Supply credentials only to jobs trusted to write to the cache. The
script writes the header into the runner's `~/.bazelrc`; do not print or upload
that file.

A failing configured cache does not necessarily fall back to local work. See
AvalancheGo's [gRPC cache rationale](https://github.com/ava-labs/avalanchego/blob/master/docs/bazel.md#why-the-remote-cache-uses-grpc)
for transport choices and timeout behavior.

## Changing the setup safely

Keep the preparation fetch list aligned with the Bazel CI commands. Metadata
commands can require external tools that fetching the FFI test target does not
cover. Keep those tools unless download-disabled validation proves they are no
longer needed.

After changing dependencies, run the
[metadata and FFI checks](../../../ffi/README.md#bazel-source-builds). After
changing cache preparation or configuration, run this from the repository root:

```sh
./scripts/run-just.sh ci-check-bazel-cache
```

This check prepares dependencies in empty input caches, then checks metadata,
toolchain alignment, and the FFI build and tests in a separate fresh output base
with dependency downloads and remote caching disabled. It proves that preparation
supplies inputs for local compilation, not that a remote cache works. It downloads
and rebuilds dependencies, so it is not part of normal pre-push checks.

After changing the Actions restore/save logic, also test CI with an exact cache
hit and with an older cache restored through the fallback key.

Remote-cache changes require working credentials and separate validation:

- Run two identical builds in fresh output bases against an initially empty
  remote cache namespace. Confirm uploads from the first and remote hits on the
  second using execution logs or server metrics—not just build success.
- Confirm builds and tests also succeed without remote-cache credentials.

Keep authorization headers out of validation diagnostics.

## Shared background

Firewood follows AvalancheGo's input-cache approach. These guides explain shared
concepts; Firewood's commands, fetch list, and cache-save policy remain local:

- [Go toolchains and dependency caches](https://github.com/ava-labs/avalanchego/blob/master/docs/go-toolchains-and-caches.md):
  compiler requirements, cache boundaries, and download restrictions.
- [Bazel CI external dependency caching](https://github.com/ava-labs/avalanchego/blob/master/docs/bazel.md#bazel-ci-external-dependency-caching):
  motivation, cache keys, explicit tool dependencies, and maintenance checks.
