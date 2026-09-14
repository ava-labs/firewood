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

if [ "$UNAME_OS" != linux ] || [ "$UNAME_ARCH" != x86_64 ]; then
	echo "Unsupported platform: $UNAME_OS/$UNAME_ARCH" >&2
	echo "Pinned benchmark Go install currently supports linux/x86_64." >&2
	exit 1
fi

# Build tarball name and URL
TARBALL="go${FIREWOOD_GO_VERSION}.linux-amd64.tar.gz"
URL="https://go.dev/dl/${TARBALL}"

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
printf '%s  %s\n' "$FIREWOOD_GO_LINUX_AMD64_SHA256" "$TARBALL" | sha256sum -c -

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

echo "Go $FIREWOOD_GO_VERSION installed to $INSTALL_DIR"
echo "Add to PATH if needed:"
echo "   export PATH=\$PATH:/usr/local/go/bin"
