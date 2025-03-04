package firewood

import (
	"bytes"
	"fmt"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"testing"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func TestMain(m *testing.M) {
	// The cgocheck debugging flag checks that all pointers are pinned.
	// TODO(arr4n) why doesn't `//go:debug cgocheck=1` work? https://go.dev/doc/godebug
	debug := strings.Split(os.Getenv("GODEBUG"), ",")
	var hasCgoCheck bool
	for _, kv := range debug {
		switch strings.TrimSpace(kv) {
		case "cgocheck=1":
			hasCgoCheck = true
			break
		case "cgocheck=0":
			fmt.Fprint(os.Stderr, "GODEBUG=cgocheck=0; MUST be 1 for Firewood cgo tests")
			os.Exit(1)
		}
	}

	if !hasCgoCheck {
		debug = append(debug, "cgocheck=1")
	}
	os.Setenv("GODEBUG", strings.Join(debug, ","))

	os.Exit(m.Run())
}

func newTestDatabase(t *testing.T) *Database {
	t.Helper()

	conf := DefaultConfig()
	conf.Create = true
	// The TempDir directory is automatically cleaned up so there's no need to
	// remove test.db.
	dbFile := filepath.Join(t.TempDir(), "test.db")

	f, err := New(dbFile, conf)
	require.NoErrorf(t, err, "NewDatabase(%+v)", conf)
	// Close() always returns nil, its signature returning an error only to
	// conform with an externally required interface.
	t.Cleanup(func() { f.Close() })
	return f
}

func TestInsert(t *testing.T) {
	f := newTestDatabase(t)
	const (
		key = "abc"
		val = "def"
	)
	f.Batch([]KeyValue{
		{[]byte(key), []byte(val)},
	})

	got, err := f.Get([]byte(key))
	require.NoErrorf(t, err, "%T.Get(%q)", f, key)
	assert.Equal(t, val, string(got), "Recover lone batch-inserted value")
}

func keyForTest(i int) []byte {
	return []byte("key" + strconv.Itoa(i))
}

func kvForTest(i int) KeyValue {
	return KeyValue{
		Key:   keyForTest(i),
		Value: []byte("value" + strconv.Itoa(i)),
	}
}

func TestInsert100(t *testing.T) {
	f := newTestDatabase(t)

	ops := make([]KeyValue, 100)
	for i := range ops {
		ops[i] = kvForTest(i)
	}
	f.Batch(ops)

	for _, op := range ops {
		got, err := f.Get(op.Key)
		require.NoErrorf(t, err, "%T.Get(%q)", f, op.Key)
		// Cast as strings to improve debug messages.
		want := string(op.Value)
		assert.Equal(t, want, string(got), "Recover nth batch-inserted value")
	}

	hash := f.Root()
	assert.Lenf(t, hash, 32, "%T.Root()", f)
	// we know the hash starts with 0xf8
	assert.Equalf(t, byte(0xf8), hash[0], "First byte of %T.Root()", f)
}

func TestRangeDelete(t *testing.T) {
	f := newTestDatabase(t)
	ops := make([]KeyValue, 100)
	for i := range ops {
		ops[i] = kvForTest(i)
	}
	f.Batch(ops)

	const deletePrefix = 1
	f.Batch([]KeyValue{{
		Key: keyForTest(deletePrefix),
		// delete all keys that start with "key1"
		Value: nil,
	}})

	for _, op := range ops {
		got, err := f.Get(op.Key)
		require.NoError(t, err)

		var want []byte
		if !bytes.HasPrefix(op.Key, keyForTest(deletePrefix)) {
			want = op.Value
		}
		assert.Equal(t, want, got)
	}
}

func TestInvariants(t *testing.T) {
	f := newTestDatabase(t)

	assert.Equalf(t, make([]byte, 32), f.Root(), "%T.Root() of empty trie")

	got, err := f.Get([]byte("non-existent"))
	require.NoError(t, err)
	assert.Nilf(t, got, "%T.Get([non-existent key])", f)
}
