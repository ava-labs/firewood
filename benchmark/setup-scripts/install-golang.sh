#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=infra/toolchains/firewood-toolchain.sh
. "$SCRIPT_DIR/../../infra/toolchains/firewood-toolchain.sh"

INSTALL_DIR="/usr/local/go"

# Check write permission
if [ -d "$INSTALL_DIR" ]; then
    if [ ! -w "$INSTALL_DIR" ]; then
        echo "Error: $INSTALL_DIR exists but is not writable." >&2
        exit 1
    fi
else
    if [ ! -w "$(dirname "$INSTALL_DIR")" ]; then
        echo "Error: Cannot create $INSTALL_DIR. $(dirname "$INSTALL_DIR") is not writable." >&2
        exit 1
    fi
fi

# Detect platform
UNAME_OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
UNAME_ARCH="$(uname -m)"

# Map to Go arch
case "$UNAME_ARCH" in
    x86_64) ARCH="amd64" ;;
    aarch64 | arm64) ARCH="arm64" ;;
    armv6l | armv7l) ARCH="arm" ;;
    *) echo "Unsupported architecture: $UNAME_ARCH" >&2; exit 1 ;;
esac

# Map to Go OS
case "$UNAME_OS" in
    linux | darwin) OS="$UNAME_OS" ;;
    *) echo "Unsupported OS: $UNAME_OS" >&2; exit 1 ;;
esac

# Build tarball name and URL
TARBALL="go${FIREWOOD_GO_VERSION}.${OS}-${ARCH}.tar.gz"
URL="https://go.dev/dl/${TARBALL}"

# The toolchain manifest pins a checksum per platform it has been verified on.
SHA_VAR="FIREWOOD_GO_$(printf '%s_%s' "$OS" "$ARCH" | tr '[:lower:]' '[:upper:]')_SHA256"
EXPECTED_SHA256="${!SHA_VAR:-}"

# Validate URL
echo "Checking URL: $URL"
if ! curl --head --fail --silent "$URL" >/dev/null; then
    echo "Error: Go tarball not found at $URL" >&2
    exit 1
fi

# Download and install
TMP_DIR=$(mktemp -d)
cd "$TMP_DIR"
echo "Downloading $TARBALL..."
curl -fLO "$URL"

if [ -n "$EXPECTED_SHA256" ]; then
    if command -v sha256sum >/dev/null 2>&1; then
        printf '%s  %s\n' "$EXPECTED_SHA256" "$TARBALL" | sha256sum -c -
    else
        printf '%s  %s\n' "$EXPECTED_SHA256" "$TARBALL" | shasum -a 256 -c -
    fi
else
    echo "Warning: no checksum pinned for ${OS}/${ARCH}; skipping verification." >&2
fi

# Validate archive format
if ! file "$TARBALL" | grep -q 'gzip compressed data'; then
    echo "Error: Downloaded file is not a valid tar.gz archive." >&2
    exit 1
fi

echo "Removing any existing Go installation in $INSTALL_DIR..."
rm -rf "$INSTALL_DIR"

echo "Extracting Go to $INSTALL_DIR..."
tar -C "$(dirname "$INSTALL_DIR")" -xzf "$TARBALL"

rm -rf "$TMP_DIR"

echo "✅ Go $FIREWOOD_GO_VERSION installed to $INSTALL_DIR"
echo "➕ Add to PATH if needed:"
echo "   export PATH=\$PATH:/usr/local/go/bin"
