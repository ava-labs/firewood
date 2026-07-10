// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package ffi

import (
	"encoding/hex"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/stretchr/testify/require"
)

const (
	cchainReplayLogPathEnv      = "CCHAIN_REPLAY_LOG_PATH"
	cchainReplayExpectedRootEnv = "CCHAIN_REPLAY_EXPECTED_ROOT"
	cchainReplayRequiredEnv     = "CCHAIN_REPLAY_REQUIRED"
)

type cchainReplayParams struct {
	logPath      string
	expectedRoot Hash
}

// cchainReplayParamsFromEnv reads C-Chain replay configuration from the environment.
// CCHAIN_REPLAY_REQUIRED gates the replay: when it is unset, the returned boolean is
// false and no other variables are read. When it is set, CCHAIN_REPLAY_LOG_PATH must name
// the replay log and CCHAIN_REPLAY_EXPECTED_ROOT must hold the root asserted once the
// replay completes.
func cchainReplayParamsFromEnv() (cchainReplayParams, bool, error) {
	if os.Getenv(cchainReplayRequiredEnv) == "" {
		return cchainReplayParams{}, false, nil
	}

	params := cchainReplayParams{logPath: os.Getenv(cchainReplayLogPathEnv)}
	if params.logPath == "" {
		return params, true, fmt.Errorf("%s must be set when %s is set", cchainReplayLogPathEnv, cchainReplayRequiredEnv)
	}

	expectedRootHex := os.Getenv(cchainReplayExpectedRootEnv)
	if expectedRootHex == "" {
		return params, true, fmt.Errorf("%s must be set when %s is set", cchainReplayExpectedRootEnv, cchainReplayLogPathEnv)
	}

	expectedRoot, err := hex.DecodeString(expectedRootHex)
	if err != nil {
		return params, true, fmt.Errorf("decode %s: %w", cchainReplayExpectedRootEnv, err)
	}
	if len(expectedRoot) != RootLength {
		return params, true, fmt.Errorf("%s must encode a %d-byte root, got %d bytes", cchainReplayExpectedRootEnv, RootLength, len(expectedRoot))
	}
	copy(params.expectedRoot[:], expectedRoot)

	return params, true, nil
}

// TestCChainReplayParamsFromEnvNotRequired verifies that when CCHAIN_REPLAY_REQUIRED
// is unset, replay is not required and the remaining variables are ignored.
func TestCChainReplayParamsFromEnvNotRequired(t *testing.T) {
	r := require.New(t)
	t.Setenv(cchainReplayRequiredEnv, "")
	t.Setenv(cchainReplayLogPathEnv, "/tmp/cchain.replay")
	t.Setenv(cchainReplayExpectedRootEnv, strings.Repeat("ab", RootLength))

	_, required, err := cchainReplayParamsFromEnv()
	r.NoError(err)
	r.False(required)
}

// TestCChainReplayParamsFromEnvValid verifies that a fully configured environment
// round-trips into the returned params: the log path is preserved and the expected
// root decodes to the exact hash bytes.
func TestCChainReplayParamsFromEnvValid(t *testing.T) {
	r := require.New(t)
	wantRootHex := "00112233445566778899aabbccddeefffedcba98765432100123456789abcdef"
	wantRootBytes, err := hex.DecodeString(wantRootHex)
	r.NoError(err)
	var wantRoot Hash
	copy(wantRoot[:], wantRootBytes)

	const logPath = "/tmp/cchain.replay"
	t.Setenv(cchainReplayRequiredEnv, "1")
	t.Setenv(cchainReplayLogPathEnv, logPath)
	t.Setenv(cchainReplayExpectedRootEnv, wantRootHex)

	params, required, err := cchainReplayParamsFromEnv()
	r.NoError(err)
	r.True(required)
	r.Equal(logPath, params.logPath)
	r.Equal(wantRoot, params.expectedRoot)
}

// TestCChainReplayParamsFromEnvRootErrors verifies that when replay is required, a
// missing or malformed expected root is an error rather than a skip.
func TestCChainReplayParamsFromEnvRootErrors(t *testing.T) {
	validRootHex := strings.Repeat("ab", RootLength)

	tests := []struct {
		name            string
		expectedRootHex string
	}{
		{
			name: "root unset",
		},
		{
			name:            "root with 0x prefix",
			expectedRootHex: "0x" + validRootHex,
		},
		{
			name:            "odd-length root",
			expectedRootHex: validRootHex[:len(validRootHex)-1],
		},
		{
			name:            "31-byte root",
			expectedRootHex: strings.Repeat("ab", RootLength-1),
		},
		{
			name:            "33-byte root",
			expectedRootHex: strings.Repeat("ab", RootLength+1),
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			r := require.New(t)
			t.Setenv(cchainReplayRequiredEnv, "1")
			t.Setenv(cchainReplayLogPathEnv, "/tmp/cchain.replay")
			t.Setenv(cchainReplayExpectedRootEnv, tt.expectedRootHex)

			_, required, err := cchainReplayParamsFromEnv()
			r.True(required)
			r.Error(err)
		})
	}
}

// TestCChainReplay replays a recorded C-Chain execution log and verifies its state root.
func TestCChainReplay(t *testing.T) {
	r := require.New(t)

	params, required, err := cchainReplayParamsFromEnv()
	if !required {
		t.Skipf("%s not set; skipping C-Chain replay", cchainReplayRequiredEnv)
	}
	r.NoError(err)

	db := newTestDatabase(t, WithTruncate(true))
	initialRoot := db.Root()
	r.Equal(
		emptyEthhashRoot,
		hex.EncodeToString(initialRoot[:]),
		"TestCChainReplay requires the FFI library built with the ethhash feature",
	)

	logs, err := loadReplayLogs(filepath.Clean(params.logPath), 0)
	r.NoError(err, "load C-Chain replay log")
	r.NotEmpty(logs, "expected at least one replay segment")

	_, err = applyReplayLogs(db, logs, replayConfig{VerifyHashes: true})
	r.NoError(err, "replay C-Chain execution log")

	root := db.Root()
	r.Equal(params.expectedRoot, root, "final C-Chain state root")
}
