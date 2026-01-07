// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package ffi

import (
	"bytes"
	"encoding/binary"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strconv"
	"testing"
	"time"

	"github.com/stretchr/testify/require"
	"github.com/vmihailenco/msgpack/v5"
)

const (
	// replayPathEnv is the environment variable that controls recording.
	// When set, FFI operations are recorded to the specified path.
	// This must match REPLAY_PATH_ENV in ffi/src/replay.rs.
	replayPathEnv = "FIREWOOD_BLOCK_REPLAY_PATH"

	// replayLogEnv is the environment variable for the replay log path.
	replayLogEnv = "REPLAY_LOG"

	// replayMaxCommitsEnv is the environment variable for limiting
	// number of executed commits.
	// If empty, defaults to 10000. Set to 0 for unlimited.
	replayMaxCommitsEnv = "REPLAY_MAX_COMMITS"
)

// ReplayLog mirrors the Rust ReplayLog type.
type ReplayLog struct {
	Operations []DbOperation `msgpack:"operations"`
}

// DbOperation is the Go representation of the Rust DbOperation enum.
// msgpack will populate exactly one of the pointer fields below.
type DbOperation struct {
	GetLatest         *GetLatest         `msgpack:"GetLatest,omitempty"`
	GetFromProposal   *GetFromProposal   `msgpack:"GetFromProposal,omitempty"`
	GetFromRoot       *GetFromRoot       `msgpack:"GetFromRoot,omitempty"`
	Batch             *Batch             `msgpack:"Batch,omitempty"`
	ProposeOnDB       *ProposeOnDB       `msgpack:"ProposeOnDB,omitempty"`
	ProposeOnProposal *ProposeOnProposal `msgpack:"ProposeOnProposal,omitempty"`
	Commit            *Commit            `msgpack:"Commit,omitempty"`
}

// GetLatest represents a read from the latest revision.
type GetLatest struct {
	Key []byte `msgpack:"key"`
}

// GetFromProposal represents a read from an uncommitted proposal.
type GetFromProposal struct {
	ProposalID uint64 `msgpack:"proposal_id"`
	Key        []byte `msgpack:"key"`
}

// GetFromRoot represents a read from a specific historical root.
type GetFromRoot struct {
	Root []byte `msgpack:"root"`
	Key  []byte `msgpack:"key"`
}

// KeyValueOp represents a single key/value mutation.
type KeyValueOp struct {
	Key   []byte `msgpack:"key"`
	Value []byte `msgpack:"value"` // nil represents delete-range
}

// Batch represents a batch operation that commits immediately.
type Batch struct {
	Pairs []KeyValueOp `msgpack:"pairs"`
}

// ProposeOnDB represents a proposal created on the database.
type ProposeOnDB struct {
	Pairs              []KeyValueOp `msgpack:"pairs"`
	ReturnedProposalID uint64       `msgpack:"returned_proposal_id"`
}

// ProposeOnProposal represents a proposal created on another proposal.
type ProposeOnProposal struct {
	ProposalID         uint64       `msgpack:"proposal_id"`
	Pairs              []KeyValueOp `msgpack:"pairs"`
	ReturnedProposalID uint64       `msgpack:"returned_proposal_id"`
}

// Commit represents a commit operation for a proposal.
type Commit struct {
	ProposalID   uint64 `msgpack:"proposal_id"`
	ReturnedHash []byte `msgpack:"returned_hash"` // nil when absent
}

// TestReplayLogExecution reads a length-prefixed MessagePack replay log
// and replays it against a fresh Firewood database using the Go FFI bindings.
//
// Environment variables:
//   - REPLAY_LOG: path to the replay log (required)
//   - REPLAY_MAX_COMMITS: max commits to replay (default: 10000, 0 for unlimited)
func TestReplayLogExecution(t *testing.T) {
	r := require.New(t)

	logPath := os.Getenv(replayLogEnv)
	if logPath == "" {
		t.Skipf("%s not set; skipping replay execution test", replayLogEnv)
	}

	maxCommits := 10000
	if v := os.Getenv(replayMaxCommitsEnv); v != "" {
		if n, err := strconv.Atoi(v); err == nil {
			maxCommits = n
		}
	}

	logs, err := loadReplayLogs(filepath.Clean(logPath), maxCommits)
	if err != nil {
		t.Skipf("unable to read replay log %q: %v", logPath, err)
	}
	r.NotEmpty(logs, "expected at least one replay segment")

	db := newTestDatabase(t)

	start := time.Now()
	commits, err := applyReplayLogs(db, logs, ReplayConfig{MaxCommits: maxCommits})
	r.NoError(err, "replay logs against database")
	elapsed := time.Since(start)

	root, err := db.Root()
	r.NoError(err, "get root after replay")
	r.NotEqual(EmptyRoot, root, "root should not be EmptyRoot after replay")

	t.Logf("Replay completed in %v (%d commits), final root: %x", elapsed, commits, root)

	r.NoError(db.Close(oneSecCtx(t)))
}

// BenchmarkReplayLog benchmarks the replay of recorded operations.
//
// Environment variables:
//   - REPLAY_LOG: path to the replay log (required)
//   - REPLAY_MAX_COMMITS: max commits to replay (default: 10000, 0 for unlimited)
//
// Run with: REPLAY_LOG=/path/to/log go test -bench=BenchmarkReplayLog -benchtime=1x
func BenchmarkReplayLog(b *testing.B) {
	r := require.New(b)
	logPath := os.Getenv(replayLogEnv)
	if logPath == "" {
		b.Skipf("%s not set; skipping replay benchmark", replayLogEnv)
	}

	maxCommits := 10000
	if v := os.Getenv(replayMaxCommitsEnv); v != "" {
		if n, err := strconv.Atoi(v); err == nil {
			maxCommits = n
		}
	}

	logs, err := loadReplayLogs(filepath.Clean(logPath), maxCommits)
	if err != nil {
		b.Skipf("unable to read replay log: %v", err)
	}
	r.NotEmpty(logs, "expected at least one replay segment")

	commits := 0
	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		b.StopTimer()
		db, err := New(b.TempDir(), WithTruncate(true))
		r.NoError(err, "create database")
		b.StartTimer()

		commits, err = applyReplayLogs(db, logs, ReplayConfig{MaxCommits: maxCommits})
		r.NoError(err, "replay failed")

		b.StopTimer()
		_ = db.Close(oneSecCtx(b))
	}

	b.ReportMetric(float64(commits), "commits")
}

// TestBlockReplayRoundTrip records operations, then replays them to a new database.
func TestBlockReplayRoundTrip(t *testing.T) {
	r := require.New(t)

	replayLogPath := filepath.Join(t.TempDir(), "roundtrip.log")

	t.Setenv(replayPathEnv, replayLogPath)

	// Phase 1: Record operations
	db1, err := New(t.TempDir(), WithTruncate(true))
	r.NoError(err)

	keys, vals := kvForTest(20)

	p1, err := db1.Propose(keys[:10], vals[:10])
	r.NoError(err)
	r.NoError(p1.Commit())

	p2, err := db1.Propose(keys[10:], vals[10:])
	r.NoError(err)
	r.NoError(p2.Commit())

	originalRoot, err := db1.Root()
	r.NoError(err)

	r.NoError(FlushBlockReplay())
	r.NoError(db1.Close(oneSecCtx(t)))

	// Verify log was created
	_, err = os.Stat(replayLogPath)
	if os.IsNotExist(err) {
		t.Skip("Replay log not created - block-replay feature may not be enabled")
	}
	r.NoError(err)

	// Phase 2: Replay to new database
	r.NoError(os.Unsetenv(replayPathEnv)) // Don't record replay operations

	logs, err := loadReplayLogs(replayLogPath, 0)
	r.NoError(err)
	r.NotEmpty(logs)

	db2, err := New(t.TempDir(), WithTruncate(true))
	r.NoError(err)

	_, err = applyReplayLogs(db2, logs, ReplayConfig{VerifyHashes: true})
	r.NoError(err)

	replayedRoot, err := db2.Root()
	r.NoError(err)

	r.Equal(originalRoot, replayedRoot, "replayed database should have same root hash")

	r.NoError(db2.Close(oneSecCtx(t)))
}

// decodeReplayLogs decodes length-prefixed MessagePack segments from data.
// If maxCommits > 0, stops loading once enough commits are found.
func decodeReplayLogs(data []byte, maxCommits int) ([]ReplayLog, error) {
	var logs []ReplayLog
	buf := bytes.NewReader(data)
	totalCommits := 0

	for {
		var segLen uint64
		if err := binary.Read(buf, binary.LittleEndian, &segLen); err != nil {
			if err == io.EOF {
				break
			}
			return nil, fmt.Errorf("reading segment length: %w", err)
		}

		if segLen == 0 {
			continue
		}
		if uint64(buf.Len()) < segLen {
			return nil, fmt.Errorf("segment length %d exceeds remaining buffer %d", segLen, buf.Len())
		}

		seg := make([]byte, segLen)
		if _, err := io.ReadFull(buf, seg); err != nil {
			return nil, fmt.Errorf("reading segment payload: %w", err)
		}

		var log ReplayLog
		if err := msgpack.Unmarshal(seg, &log); err != nil {
			return nil, fmt.Errorf("decoding segment as ReplayLog: %w", err)
		}
		logs = append(logs, log)

		// Check if we have enough commits
		if maxCommits > 0 {
			for _, op := range log.Operations {
				if op.Commit != nil {
					totalCommits++
				}
			}
			if totalCommits >= maxCommits {
				break
			}
		}
	}

	return logs, nil
}

// ReplayConfig controls replay behavior.
type ReplayConfig struct {
	// MaxCommits limits the number of commits to replay. 0 means unlimited.
	MaxCommits int
	// VerifyHashes enables verification of returned hashes after commits.
	VerifyHashes bool
}

// applyReplayLogs applies replay logs to a database.
// Returns the number of commits applied and any error encountered.
func applyReplayLogs(db *Database, logs []ReplayLog, cfg ReplayConfig) (int, error) {
	proposals := make(map[uint64]*Proposal)
	totalCommits := 0

	for _, segment := range logs {
		for _, op := range segment.Operations {
			switch {
			case op.GetLatest != nil:
				// Read operations - errors are non-fatal during replay
				_, _ = db.Get(op.GetLatest.Key)

			case op.GetFromRoot != nil:
				root, err := bytesToHash(op.GetFromRoot.Root)
				if err == nil {
					_, _ = db.GetFromRoot(root, op.GetFromRoot.Key)
				}

			case op.GetFromProposal != nil:
				if prop, ok := proposals[op.GetFromProposal.ProposalID]; ok {
					_, _ = prop.Get(op.GetFromProposal.Key)
				}

			case op.Batch != nil:
				keys, vals := splitPairs(op.Batch.Pairs)
				if _, err := db.Update(keys, vals); err != nil {
					return totalCommits, fmt.Errorf("Batch: %w", err)
				}

			case op.ProposeOnDB != nil:
				keys, vals := splitPairs(op.ProposeOnDB.Pairs)
				prop, err := db.Propose(keys, vals)
				if err != nil {
					return totalCommits, fmt.Errorf("ProposeOnDB: %w", err)
				}
				proposals[op.ProposeOnDB.ReturnedProposalID] = prop

			case op.ProposeOnProposal != nil:
				parent, ok := proposals[op.ProposeOnProposal.ProposalID]
				if !ok {
					return totalCommits, fmt.Errorf("ProposeOnProposal: unknown parent proposal id %d", op.ProposeOnProposal.ProposalID)
				}
				keys, vals := splitPairs(op.ProposeOnProposal.Pairs)
				prop, err := parent.Propose(keys, vals)
				if err != nil {
					return totalCommits, fmt.Errorf("ProposeOnProposal: %w", err)
				}
				proposals[op.ProposeOnProposal.ReturnedProposalID] = prop

			case op.Commit != nil:
				prop, ok := proposals[op.Commit.ProposalID]
				if !ok {
					return totalCommits, fmt.Errorf("Commit: unknown proposal id %d", op.Commit.ProposalID)
				}
				delete(proposals, op.Commit.ProposalID)
				if err := prop.Commit(); err != nil {
					return totalCommits, fmt.Errorf("Commit: %w", err)
				}

				if cfg.VerifyHashes && op.Commit.ReturnedHash != nil {
					root, err := db.Root()
					if err != nil {
						return totalCommits, fmt.Errorf("Root after Commit: %w", err)
					}
					if !bytes.Equal(op.Commit.ReturnedHash, root[:]) {
						return totalCommits, fmt.Errorf("root hash mismatch: expected %x, got %x", op.Commit.ReturnedHash, root[:])
					}
				}
				totalCommits++
				if cfg.MaxCommits > 0 && totalCommits >= cfg.MaxCommits {
					return totalCommits, nil
				}

			default:
				return totalCommits, fmt.Errorf("unknown or empty DbOperation: %+v", op)
			}
		}
	}

	return totalCommits, nil
}

// loadReplayLogs reads and decodes replay logs from a file path.
// If maxCommits > 0, stops loading once enough commits are found.
func loadReplayLogs(logPath string, maxCommits int) ([]ReplayLog, error) {
	data, err := os.ReadFile(logPath)
	if err != nil {
		return nil, err
	}
	return decodeReplayLogs(data, maxCommits)
}

func splitPairs(pairs []KeyValueOp) ([][]byte, [][]byte) {
	keys := make([][]byte, 0, len(pairs))
	vals := make([][]byte, 0, len(pairs))
	for _, p := range pairs {
		keys = append(keys, p.Key)
		vals = append(vals, p.Value)
	}
	return keys, vals
}

func bytesToHash(b []byte) (Hash, error) {
	if len(b) != RootLength {
		return Hash{}, fmt.Errorf("expected root length %d, got %d", RootLength, len(b))
	}
	var h Hash
	copy(h[:], b)
	return h, nil
}
