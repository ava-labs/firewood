// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package ffi

import (
	"bytes"
	"encoding/binary"
	"io"
	"os"
	"path/filepath"
	"testing"

	"github.com/stretchr/testify/require"
)

// TestBlockReplayIntegration exercises a broad sequence of operations that are
// recorded by the block replay feature and asserts that the replay log is
// flushed to disk in a length-prefixed format.
func TestBlockReplayIntegration(t *testing.T) {
	r := require.New(t)

// 	logPath := filepath.Join(t.TempDir(), "block_replay.log")
	logPath := filepath.Join("block_replay.log")
	t.Setenv("FIREWOOD_BLOCK_REPLAY_PATH", logPath)

	db := newTestDatabase(t)

	// 1. Batch update via Database.Update (fwd_batch).
	keys1, vals1 := kvForTest(50)
	root1, err := db.Update(keys1, vals1)
	r.NoError(err, "Update initial batch")

	// 2. Get via latest (fwd_get_latest).
	for i := 0; i < 10; i++ {
		got, err := db.Get(keys1[i])
		r.NoError(err, "Get(%d)", i)
		r.Equal(vals1[i], got, "Get(%d) mismatch", i)
	}

	// 3. Get via root (fwd_get_from_root).
	for i := 10; i < 20; i++ {
		got, err := db.GetFromRoot(root1, keys1[i])
		r.NoError(err, "GetFromRoot(%d)", i)
		r.Equal(vals1[i], got, "GetFromRoot(%d) mismatch", i)
	}

	// 4. Propose on DB (fwd_propose_on_db), get from proposal (fwd_get_from_proposal),
	// and commit (fwd_commit_proposal).
	keys2, vals2 := kvForTest(20)
	proposal, err := db.Propose(keys2, vals2)
	r.NoError(err, "Propose on DB")

	for i := 0; i < 10; i++ {
		got, err := proposal.Get(keys2[i])
		r.NoError(err, "Proposal.Get(%d)", i)
		r.Equal(vals2[i], got, "Proposal.Get(%d) mismatch", i)
	}

	// 5. Propose on proposal (fwd_propose_on_proposal).
	keys3, vals3 := kvForTest(10)
	proposal2, err := proposal.Propose(keys3, vals3)
	r.NoError(err, "Propose on Proposal")

	// Check that chained proposal sees data from its parent.
	for i := 0; i < len(keys2); i++ {
		got, err := proposal2.Get(keys2[i])
		r.NoError(err, "Proposal2.Get(%d)", i)
		r.Equal(vals2[i], got, "Proposal2.Get(%d) mismatch", i)
	}

	r.NoError(proposal.Commit(), "Commit first proposal")
	r.NoError(proposal2.Commit(), "Commit second proposal")

	// 6. Verify committed data via latest queries.
	for i := 0; i < len(keys2); i++ {
		got, err := db.Get(keys2[i])
		r.NoError(err, "Get after commit (%d)", i)
		r.Equal(vals2[i], got, "Get after commit (%d) mismatch", i)
	}

	// Force a flush of whatever is still buffered, regardless of threshold.
	err = FlushStuff()
	r.NoError(err, "fwd_block_replay_flush")

	data, err := os.ReadFile(logPath)
	r.NoError(err, "reading block replay log")
	r.NotEmpty(data, "block replay log should not be empty")

	// The log is written as a sequence of segments:
	// [len: u64 LE][bytes][len: u64 LE][bytes]...
	buf := bytes.NewReader(data)
	var segments int

	for {
		var segLen uint64
		if err := binary.Read(buf, binary.LittleEndian, &segLen); err != nil {
			if err == io.EOF {
				break
			}
			r.NoError(err, "reading segment length")
		}

		r.Greater(segLen, uint64(0), "segment length must be > 0")
		if uint64(buf.Len()) < segLen {
			t.Fatalf("segment length %d exceeds remaining buffer %d", segLen, buf.Len())
		}

		// Skip the segment payload.
		if _, err := buf.Seek(int64(segLen), io.SeekCurrent); err != nil {
			r.NoError(err, "seeking segment payload")
		}

		segments++
	}

	r.Greater(segments, 0, "expected at least one segment in replay log")
}

