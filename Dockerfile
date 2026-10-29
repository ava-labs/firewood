# Builds an avalanchego image whose firewood dependency is replaced by a
# from-source build, mirroring the proven benchmark/launch flow
# (benchmark/launch/launch-stages.yaml). Both repos are cloned inside the
# build, so the build context is intentionally empty (see .dockerignore).
#
# The Go version is supplied as a build argument rather than hard-coded
# to minimize the cost of version changes. The default tracks the go
# directive in avalanchego's go.mod and firewood's ffi/go.mod.
ARG GO_VERSION=1.25.10

# ============= Compilation Stage ================
# This image is native-only: the firewood FFI is a static archive that must
# match the target arch, and the maxperf profile (fat LTO, codegen-units=1)
# is expensive enough without emulation, so the cross-compilation machinery
# from avalanchego's Dockerfile is deliberately not ported.
FROM golang:${GO_VERSION}-bookworm AS builder

# Install the rust toolchain used to build the firewood FFI. The fixed
# RUSTUP_HOME/CARGO_HOME location matches firewood's launch tooling.
# RUST_VERSION accepts anything rustup does (e.g. 1.94.0 to pin the MSRV).
ARG RUST_VERSION=stable
ENV RUSTUP_HOME=/usr/local/rust \
    CARGO_HOME=/usr/local/rust \
    PATH=/usr/local/rust/bin:${PATH}
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --no-modify-path --profile minimal --default-toolchain "${RUST_VERSION}"

# The go module replacements below use relative paths that assume firewood
# and avalanchego are siblings of each other.
WORKDIR /build

# Each *_COMMIT ref may be a branch, a tag, or a full (not abbreviated)
# commit SHA. `git clone --branch` rejects SHAs, so fetch by ref instead
# (the actions/checkout pattern; GitHub permits fetching reachable SHAs).
# The .git directory is kept so builds can stamp the real commit via
# `git rev-parse HEAD`.
#
# NOTE: when a ref is a branch name, docker's layer cache will not notice
# new commits on that branch; pin a SHA or build with --no-cache to refresh.
ARG FIREWOOD_URL=https://github.com/ava-labs/firewood.git
ARG FIREWOOD_COMMIT=main
RUN git init --quiet firewood \
    && cd firewood \
    && git remote add origin "${FIREWOOD_URL}" \
    && git fetch --depth 1 origin "${FIREWOOD_COMMIT}" \
    && git checkout --quiet FETCH_HEAD

# Build the static FFI archive. Building from ffi/ places the archive in
# ../target/maxperf, one of the paths searched by the go package's cgo
# LDFLAGS (a fresh clone guarantees no stale target/debug shadows it).
# The launch tooling sometimes adds block-replay to the feature set.
# This is the most expensive layer; it is ordered before the avalanchego
# clone so that changing AVALANCHEGO_COMMIT does not invalidate it.
ARG FIREWOOD_FEATURES=ethhash,logger
RUN cd firewood/ffi \
    && cargo build --profile maxperf --features "${FIREWOOD_FEATURES}"

ARG AVALANCHEGO_URL=https://github.com/ava-labs/avalanchego.git
ARG AVALANCHEGO_COMMIT=master
RUN git init --quiet avalanchego \
    && cd avalanchego \
    && git remote add origin "${AVALANCHEGO_URL}" \
    && git fetch --depth 1 origin "${AVALANCHEGO_COMMIT}" \
    && git checkout --quiet FETCH_HEAD

# Point avalanchego at the local firewood FFI, mirroring the launch flow:
# replace in graft/coreth and in the repo root (avalanchego builds in
# workspace mode, and both relative paths resolve to /build/firewood/ffi,
# so the replacements agree). `go mod tidy` reads the replacement
# directory's go.mod, which is why firewood is cloned before this step;
# tidy does not need the cargo build output.
RUN cd avalanchego/graft/coreth \
    && go mod edit -replace github.com/ava-labs/firewood-go-ethhash/ffi=../../../firewood/ffi \
    && go mod tidy \
    && cd ../.. \
    && go mod edit -replace github.com/ava-labs/firewood-go-ethhash/ffi=../firewood/ffi \
    && go mod tidy

# Build avalanchego (scripts/build.sh sets CGO_ENABLED=1 and the BLST
# flags itself). AVALANCHEGO_COMMIT is force-emptied because docker
# exposes ARGs as environment variables and the ref may be a branch name;
# an empty value makes scripts/git_commit.sh fall through to
# `git rev-parse HEAD`, stamping the real commit SHA.
ARG RACE_FLAG=""
RUN cd avalanchego \
    && AVALANCHEGO_COMMIT='' ./scripts/build.sh ${RACE_FLAG}

# ============= Cleanup Stage ================
FROM debian:12-slim AS execution

# Maintain compatibility with avalanchego's published images: same binary
# location, working directory, and entrypoint. The firewood FFI and BLST
# are statically linked into the binary, so no extra runtime packages are
# required beyond debian's glibc.
COPY --from=builder /build/avalanchego/build/ /avalanchego/build/
WORKDIR /avalanchego/build

CMD ["./avalanchego"]
