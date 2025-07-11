#!/usr/bin/env bash

curl -o /tmp/upstream.yml https://raw.githubusercontent.com/ava-labs/avalanchego/refs/heads/master/.golangci.yml
    
# Generate diff
diff /tmp/upstream.yml .golangci.yaml > /tmp/diff.txt || true

# Compare with expected diff
if ! diff /tmp/diff.txt .golangci_yaml_expected_changes.txt; then
    echo ".golangci.yaml has unexpected changes from AvalancheGo"
    exit 1
fi
