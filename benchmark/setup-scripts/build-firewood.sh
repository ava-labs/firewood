#!/usr/bin/env bash

# Firewood Builder
#
# Builds or fetches Firewood FFI library from either pre-built binaries or source.
#
# Prerequisites:
#   - Rust toolchain (if building from source)
#   - Linux: build-essential package
#   - macOS: Xcode command line tools
#
# Usage:
#   ./build_firewood.sh <version> [options]
#
# Arguments:
#   version       Firewood version
#                 - Format 'ffi/vX.Y.Z' for pre-built binaries
#                 - Commit hash or branch name for source build
#
# Options:
#   --workspace PATH    Custom workspace directory (default: ./firewood-workspace)
#   --target TRIPLE     Target platform triple (auto-detected if not specified)
#   --features LIST     Cargo features, comma-separated (default: ethhash,logger)
#   --profile NAME      Cargo build profile (default: maxperf)
#
# Output:
#   Prints absolute FFI path to stdout on success
#   Progress messages printed to stderr
#   Exit code 0 on success, non-zero on failure

set -euo pipefail

show_help() {
    cat >&2 << 'EOF'
Firewood FFI Builder

Builds or fetches Firewood FFI library from either pre-built binaries or source.

USAGE:
    build_firewood.sh <version> [options]
    build_firewood.sh --help

ARGUMENTS:
    <version>           Firewood version
                        - Format 'ffi/vX.Y.Z' for pre-built binaries
                        - Commit hash or branch name for source build

OPTIONS:
    --workspace PATH    Custom workspace directory
                        Default: ./firewood-workspace

    --target TRIPLE     Target platform triple
                        Auto-detected if not specified
                        Supported: x86_64-unknown-linux-gnu,
                                   aarch64-unknown-linux-gnu,
                                   x86_64-apple-darwin,
                                   aarch64-apple-darwin

    --features LIST     Cargo features (comma-separated)
                        Default: ethhash,logger
                        Only used for source builds

    --profile NAME      Cargo build profile
                        Default: maxperf
                        Only used for source builds

    --help, -h          Show this help message

PREREQUISITES:
    - Rust toolchain (if building from source)
    - Linux: build-essential package
    - macOS: Xcode command line tools

OUTPUT:
    Prints absolute FFI path to stdout on success
    Progress messages printed to stderr
    Exit code 0 on success, non-zero on failure

EXAMPLES:
    # Use pre-built binaries (fast)
    ./build_firewood.sh ffi/v1.2.3

    # Build from source at main branch
    ./build_firewood.sh main

    # Build from specific commit
    ./build_firewood.sh abc123def

    # Custom workspace location
    ./build_firewood.sh ffi/v1.2.3 --workspace /tmp/firewood-build

    # Override target platform
    ./build_firewood.sh main --target aarch64-unknown-linux-gnu

    # Custom features and profile for source build
    ./build_firewood.sh main --features ethhash,logger,metrics --profile release

    # Capture FFI path in variable
    FFI_PATH=$(./build_firewood.sh ffi/v1.2.3)
    echo "FFI is at: $FFI_PATH"

EOF
}

if [ "$1" = "--help" ] || [ "$1" = "-h" ]; then
    show_help
    exit 0
fi

WORKSPACE_PATH="./firewood-workspace"
TARGET_PLATFORM=""
FEATURES="ethhash,logger"
PROFILE="maxperf"

# Parse arguments
if [ $# -lt 1 ]; then
    echo "Error: Version argument required" >&2
    echo "Usage: $0 <version> [--workspace PATH] [--target TRIPLE] [--features LIST] [--profile NAME]" >&2
    exit 1
fi

VERSION="$1"
shift

while [ $# -gt 0 ]; do
    case "$1" in
        --workspace)
            WORKSPACE_PATH="$2"
            shift 2
            ;;
        --target)
            TARGET_PLATFORM="$2"
            shift 2
            ;;
        --features)
            FEATURES="$2"
            shift 2
            ;;
        --profile)
            PROFILE="$2"
            shift 2
            ;;
        *)
            echo "Error: Unknown option $1" >&2
            exit 1
            ;;
    esac
done

# Detect target platform if not specified
if [ -z "$TARGET_PLATFORM" ]; then
    OS="$(uname -s)"
    ARCH="$(uname -m)"

    case "$OS-$ARCH" in
        Linux-x86_64)
            TARGET_PLATFORM="x86_64-unknown-linux-gnu"
            ;;
        Linux-aarch64)
            TARGET_PLATFORM="aarch64-unknown-linux-gnu"
            ;;
        Darwin-x86_64)
            TARGET_PLATFORM="x86_64-apple-darwin"
            ;;
        Darwin-arm64)
            TARGET_PLATFORM="aarch64-apple-darwin"
            ;;
        *)
            echo "Error: Unsupported platform: $OS-$ARCH" >&2
            echo "Supported: Linux-x86_64, Linux-aarch64, Darwin-x86_64, Darwin-arm64" >&2
            exit 1
            ;;
    esac
    echo "Auto-detected target: $TARGET_PLATFORM" >&2
else
    echo "Using specified target: $TARGET_PLATFORM" >&2
fi

# Clean workspace before building
if [ -d "$WORKSPACE_PATH" ]; then
    echo "Cleaning existing workspace: $WORKSPACE_PATH"
    rm -rf "$WORKSPACE_PATH"
fi

# Create workspace
mkdir -p "$WORKSPACE_PATH"
echo "Using workspace: $WORKSPACE_PATH" >&2

# Determine build type based on version format
# Pre-built binaries use 'ffi/vX.Y.Z' format, everything else is source build
if [[ "$VERSION" =~ ^ffi/ ]]; then
    BUILD_TYPE="prebuilt"
    echo "Using pre-built binaries for $VERSION" >&2
else
    BUILD_TYPE="source"
    echo "Building from source at $VERSION" >&2
fi

# === Pre-built path ===
# Leverages Firewood's official pre-built binaries repository
if [ "$BUILD_TYPE" = "prebuilt" ]; then
    echo "Fetching pre-built Firewood FFI..." >&2

    # Clone the pre-built binaries repository at specified version tag
    git clone --quiet --depth 1 --branch "$VERSION" \
        https://github.com/ava-labs/firewood-go-ethhash.git \
        "$WORKSPACE_PATH/firewood-ffi" >&2 2>&1

    # Verify library exists for target platform
    # Pre-built repository organizes libraries by platform triple
    EXPECTED_LIB="$WORKSPACE_PATH/firewood-ffi/ffi/libs/$TARGET_PLATFORM/libfirewood_ffi.a"

    if [ ! -f "$EXPECTED_LIB" ]; then
        echo "Error: Pre-built library not found for platform $TARGET_PLATFORM" >&2
        echo "Expected: $EXPECTED_LIB" >&2
        echo "Available platforms:" >&2
        find "$WORKSPACE_PATH/firewood-ffi/ffi/libs/" -name "libfirewood_ffi.a" 2>/dev/null >&2 || echo "No libraries found" >&2
        exit 1
    fi

    echo "Pre-built library verified for $TARGET_PLATFORM" >&2
fi

# === Source build path ===
if [ "$BUILD_TYPE" = "source" ]; then
    echo "Building Firewood FFI from source..." >&2

    # Set macOS deployment target
    # Required for macOS builds to ensure compatibility
    if [ "$(uname -s)" = "Darwin" ]; then
        export MACOSX_DEPLOYMENT_TARGET=15.0
        echo "Set MACOSX_DEPLOYMENT_TARGET=15.0 for macOS build" >&2
    fi

    # Clone Firewood source at specified version (commit/branch/tag)
    echo "Cloning Firewood source at $VERSION..." >&2
    git clone --quiet https://github.com/ava-labs/firewood.git \
        "$WORKSPACE_PATH/firewood-src" >&2 2>&1

    cd "$WORKSPACE_PATH/firewood-src"
    git checkout "$VERSION" >&2 2>&1

    echo "Building with profile=$PROFILE features=$FEATURES target=$TARGET_PLATFORM" >&2

    # Build the FFI library using Cargo
    # We use --profile and --features to match Firewood's build patterns
    cargo build \
        --profile "$PROFILE" \
        --features "$FEATURES" \
        --target "$TARGET_PLATFORM" \
        -p firewood-ffi >&2

    cd - > /dev/null

    # Verify build output exists
    # The profile name maps to a directory (maxperf, release, debug, etc.)
    EXPECTED_LIB="$WORKSPACE_PATH/firewood-src/target/$TARGET_PLATFORM/$PROFILE/libfirewood_ffi.a"
    if [ ! -f "$EXPECTED_LIB" ]; then
        echo "Error: Build failed - library not found at $EXPECTED_LIB" >&2
        exit 1
    fi

    echo "Creating FFI directory structure..." >&2

    # Create unified structure matching pre-built layout
    # This ensures consistent paths regardless of build type
    mkdir -p "$WORKSPACE_PATH/firewood-ffi"
    cp -r "$WORKSPACE_PATH/firewood-src/ffi" "$WORKSPACE_PATH/firewood-ffi/"

    # Copy built library to platform-specific location
    # Matches the pre-built repository structure for consistency
    mkdir -p "$WORKSPACE_PATH/firewood-ffi/ffi/libs/$TARGET_PLATFORM"
    cp "$EXPECTED_LIB" "$WORKSPACE_PATH/firewood-ffi/ffi/libs/$TARGET_PLATFORM/"
fi

# === Final verification and output ===
FFI_PATH="$WORKSPACE_PATH/firewood-ffi/ffi"

# Ensure FFI directory exists
if [ ! -d "$FFI_PATH" ]; then
    echo "Error: FFI directory not found at $FFI_PATH" >&2
    exit 1
fi

# Verify required library exists for target platform
REQUIRED_LIB="$FFI_PATH/libs/$TARGET_PLATFORM/libfirewood_ffi.a"
if [ ! -f "$REQUIRED_LIB" ]; then
    echo "Error: Required library not found at $REQUIRED_LIB" >&2
    exit 1
fi

echo "Firewood FFI ready for $TARGET_PLATFORM ($BUILD_TYPE)" >&2

# Output absolute path to stdout for consumption by other scripts
realpath "$FFI_PATH"