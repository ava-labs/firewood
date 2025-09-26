#!/usr/bin/env bash
set -euo pipefail

echo "Testing firewood flake functionality..."

# Test flake can be evaluated
echo "Checking flake evaluation..."
nix flake check

# Test package build
echo "Building firewood-ffi package..."
nix build .#firewood-ffi

# Test package outputs
echo "Checking package outputs..."
ls -la result/
ls -la result/include/
ls -la result/lib/

# Verify header and library exist
echo "Verifying package contents..."
test -f result/include/firewood.h && echo "✓ Header file exists"
test -f result/lib/libfirewood_ffi.a && echo "✓ Static library exists"

# Test development shell
echo "Testing development shell..."
nix develop --command bash -c "
  echo 'Firewood development shell loaded successfully'
  rustc --version
  cargo --version
  echo 'Development tools available'
"

# Optional: Test Go integration if Go is available on the system
if command -v go >/dev/null 2>&1; then
  echo "Testing Go integration..."
  export CGO_LDFLAGS="-L./result/lib -lfirewood_ffi"
  export CGO_CFLAGS="-I./result/include"

  # Just check that Go can see the library, don't run full tests
  go list -f '{{.Dir}}' . >/dev/null && echo "✓ Go module accessible"
  echo "CGO_LDFLAGS: $CGO_LDFLAGS"
  echo "CGO_CFLAGS: $CGO_CFLAGS"
else
  echo "Go not found, skipping Go integration test"
fi

echo "✓ Firewood flake validation complete"