package firewood

// implement a specific interface for firewood
// this is used for some of the firewood performance tests

// Validate that Firewood implements the KVBackend interface
var _ KVBackend = (*Firewood)(nil)

// Copy of KVBackend from ava-labs/avalanchego
type KVBackend interface {
	// Returns the current root hash of the trie.
	// Empty trie must return common.Hash{}.
	// Length of the returned slice must be common.HashLength.
	Root() []byte

	// Get retrieves the value for the given key.
	// If the key does not exist, it must return (nil, nil).
	Get(key []byte) ([]byte, error)

	// Prefetch loads the intermediary nodes of the given key into memory.
	// The first return value is ignored.
	Prefetch(key []byte) ([]byte, error)

	// After this call, Root() should return the same hash as returned by this call.
	// Note when length of a particular value is zero, it means the corresponding
	// key should be deleted.
	// There may be duplicate keys in the batch provided, and the last one should
	// take effect.
	// Note after this call, the next call to Update must build on the returned root,
	// regardless of whether Commit is called.
	// Length of the returned root must be common.HashLength.
	Update(keys, vals [][]byte) ([]byte, error)

	// After this call, changes related to [root] should be persisted to disk.
	// This may be implemented as no-op if Update already persists changes, or
	// commits happen on a rolling basis.
	// Length of the root slice is guaranteed to be common.HashLength.
	Commit(root []byte) error

	// Close closes the backend and releases all held resources.
	Close() error
}

// Prefetch does nothing since we don't need to prefetch for firewood
func (f *Firewood) Prefetch(key []byte) ([]byte, error) {
	return nil, nil
}

// / Commit does nothing, since update already persists changes
func (f *Firewood) Commit(root []byte) error {
	return nil
}

// Update could use some more work, but for now we just batch the keys and values
func (f *Firewood) Update(keys, vals [][]byte) ([]byte, error) {
	// batch the keys and values
	ops := make([]KeyValue, len(keys))
	for i := range keys {
		ops[i] = KeyValue{keys[i], vals[i]}
	}
	return f.Batch(ops), nil
}