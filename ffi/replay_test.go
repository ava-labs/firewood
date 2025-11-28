// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package ffi

import (
	"bytes"
	"context"
	"encoding/binary"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"testing"
	"time"

	"github.com/stretchr/testify/require"
	"github.com/vmihailenco/msgpack/v5"
)

// ReplayLog mirrors the Rust ReplayLog type.
type ReplayLog struct {
	Operations []DbOperation `msgpack:"operations"`
}

// DbOperation is the Go representation of the Rust DbOperation enum.
// It uses an externally tagged representation: each operation is encoded
// as a map with a single key (the variant name) and a value containing
// the corresponding struct fields. msgpack will populate exactly one of
// the pointer fields below.
type DbOperation struct {
	GetLatest         *GetLatest         `msgpack:"GetLatest,omitempty"`
	GetFromProposal   *GetFromProposal   `msgpack:"GetFromProposal,omitempty"`
	GetFromRoot       *GetFromRoot       `msgpack:"GetFromRoot,omitempty"`
	Batch             *Batch             `msgpack:"Batch,omitempty"`
	ProposeOnDB       *ProposeOnDB       `msgpack:"ProposeOnDB,omitempty"`
	ProposeOnProposal *ProposeOnProposal `msgpack:"ProposeOnProposal,omitempty"`
	Commit            *Commit            `msgpack:"Commit,omitempty"`
}

type GetLatest struct {
	Key []byte `msgpack:"key"`
}

type GetFromProposal struct {
	ProposalID uint64 `msgpack:"proposal_id"`
	Key        []byte `msgpack:"key"`
}

type GetFromRoot struct {
	Root []byte `msgpack:"root"`
	Key  []byte `msgpack:"key"`
}

type KeyValueOp struct {
	Key   []byte `msgpack:"key"`
	Value []byte `msgpack:"value"` // nil represents delete-range
}

type Batch struct {
	Pairs []KeyValueOp `msgpack:"pairs"`
}

type ProposeOnDB struct {
	Pairs              []KeyValueOp `msgpack:"pairs"`
	ReturnedProposalID uint64       `msgpack:"returned_proposal_id"`
}

type ProposeOnProposal struct {
	ProposalID         uint64       `msgpack:"proposal_id"`
	Pairs              []KeyValueOp `msgpack:"pairs"`
	ReturnedProposalID uint64       `msgpack:"returned_proposal_id"`
}

type Commit struct {
	ProposalID   uint64 `msgpack:"proposal_id"`
	ReturnedHash []byte `msgpack:"returned_hash"` // nil when absent
}

// TestReplayLogReexecution reads a length-prefixed MessagePack replay log
// (the output of convert_rkyv_log_to_rmp_file) and replays it against a
// fresh Firewood database using the Go FFI bindings.
//
// The path to the replay log is taken from the FIREWOOD_REPLAY_RMP_LOG
// environment variable. If it is not set, the test is skipped.
func TestReplayLogReexecution(t *testing.T) {
	t.Helper()
	r := require.New(t)

	logPath := os.Getenv("FIREWOOD_REPLAY_RMP_LOG")
	if logPath == "" {
		t.Skip("FIREWOOD_REPLAY_RMP_LOG not set; skipping replay integration test")
	}
	logPath = filepath.Clean(logPath)

	data, err := os.ReadFile(logPath)
	if err != nil {
		t.Skipf("unable to read replay log %q: %v", logPath, err)
	}

	logs, err := decodeReplayLogs(data)
	r.NoError(err, "decode replay logs")
	r.NotEmpty(logs, "expected at least one replay segment")

	r.NoError(StartMetrics())

	db := newTestDatabase(t)

	// let's see how long this takes
	start := time.Now().UnixMilli()
	err = applyReplayLogsToDatabase(t, db, logs)
	r.NoError(err, "replay logs against database")
	end := time.Now().UnixMilli()
	fmt.Printf("Replay took %d milliseconds\n", end-start)

	// Basic sanity: after replaying, root should be non-empty.
	root, err := db.Root()
	r.NoError(err, "get root after replay")
	r.NotEqual(EmptyRoot, root, "root should not be EmptyRoot after replay")

	mt, err := GatherMetrics()
	r.NoError(err, "gather metrics")

	metricPath := os.Getenv("FIREWOOD_METRICS_PATH")
	if metricPath != "" {
		mt += "# TYPE firewood_replay_spent counter\n"
		mt += fmt.Sprintf("firewood_replay_spent %d\n", end-start)
		r.NoError(os.WriteFile(metricPath, []byte(mt), 0644))
	}

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	r.NoError(db.Close(ctx))
}

func decodeReplayLogs(data []byte) ([]ReplayLog, error) {
	var logs []ReplayLog
	buf := bytes.NewReader(data)

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
	}

	return logs, nil
}

func applyReplayLogsToDatabase(t *testing.T, db *Database, logs []ReplayLog) error {
	t.Helper()
	r := require.New(t)

	proposals := make(map[uint64]*Proposal)
	total_commits := 0
	for _, segment := range logs {
		for _, op := range segment.Operations {
			switch {
			case op.GetLatest != nil:
				_, err := db.Get(op.GetLatest.Key)
				r.NoError(err, "GetLatest")

			case op.GetFromRoot != nil:
				root, err := bytesToHash(op.GetFromRoot.Root)
				if err != nil {
					return fmt.Errorf("GetFromRoot root decode: %w", err)
				}
				_, err = db.GetFromRoot(root, op.GetFromRoot.Key)
				r.NoError(err, "GetFromRoot")

			case op.GetFromProposal != nil:
				prop, ok := proposals[op.GetFromProposal.ProposalID]
				if !ok {
					return fmt.Errorf("GetFromProposal: unknown proposal id %d", op.GetFromProposal.ProposalID)
				}
				_, err := prop.Get(op.GetFromProposal.Key)
				r.NoError(err, "GetFromProposal")

			case op.Batch != nil:
				keys, vals := splitPairs(op.Batch.Pairs)
				_, err := db.Update(keys, vals)
				r.NoError(err, "Batch/Update")

			case op.ProposeOnDB != nil:
				keys, vals := splitPairs(op.ProposeOnDB.Pairs)
				id := op.ProposeOnDB.ReturnedProposalID
				prop, err := db.Propose(keys, vals)
				r.NoError(err, "ProposeOnDB")
				proposals[id] = prop

			case op.ProposeOnProposal != nil:
				parentID := op.ProposeOnProposal.ProposalID
				parent, ok := proposals[parentID]
				if !ok {
					return fmt.Errorf("ProposeOnProposal: unknown parent proposal id %d", parentID)
				}
				keys, vals := splitPairs(op.ProposeOnProposal.Pairs)
				id := op.ProposeOnProposal.ReturnedProposalID
				prop, err := parent.Propose(keys, vals)
				r.NoError(err, "ProposeOnProposal")
				proposals[id] = prop

			case op.Commit != nil:
				id := op.Commit.ProposalID
				prop, ok := proposals[id]
				if !ok {
					return fmt.Errorf("Commit: unknown proposal id %d", id)
				}
				delete(proposals, id)
				err := prop.Commit()
				r.NoError(err, "Commit")

				if op.Commit.ReturnedHash != nil {
					root, err := db.Root()
					if err == nil {
						r.Equal(op.Commit.ReturnedHash, root[:], "root hash mismatch after Commit")
					}
				}
				total_commits++
				if total_commits > 10000 {
					return nil
				}

			default:
				return fmt.Errorf("unknown or empty DbOperation: %+v", op)
			}
		}
	}

	return nil
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
