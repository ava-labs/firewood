package tests

import (
	"encoding/binary"
	"math/big"
	"path"
	"testing"

	firewood "github.com/ava-labs/firewood/ffi/v2"
	"github.com/ethereum/go-ethereum/common"
	"github.com/ethereum/go-ethereum/core/rawdb"
	"github.com/ethereum/go-ethereum/core/state"
	"github.com/ethereum/go-ethereum/core/types"
	"github.com/ethereum/go-ethereum/rlp"
	"github.com/ethereum/go-ethereum/trie"
	"github.com/stretchr/testify/require"
	"golang.org/x/crypto/sha3"
)

func hashData(input []byte) common.Hash {
	var hasher = sha3.NewLegacyKeccak256()
	var hash common.Hash
	hasher.Reset()
	hasher.Write(input)
	hasher.Sum(hash[:0])
	return hash
}

func TestInsert(t *testing.T) {
	file := path.Join(t.TempDir(), "test.db")
	cfg := firewood.DefaultConfig()
	cfg.Create = true
	db, err := firewood.New(file, cfg)
	require.NoError(t, err)
	defer db.Close()

	type storageKey struct {
		addr common.Address
		key  common.Hash
	}

	addrs := make(map[common.Address]struct{})
	storages := make(map[storageKey]struct{})

	chooseAddr := func() common.Address {
		for addr := range addrs {
			return addr
		}
		return common.Address{}
	}

	chooseStorage := func() storageKey {
		for key := range storages {
			return key
		}
		return storageKey{}
	}

	deleteStorage := func(k storageKey) {
		delete(storages, k)
	}

	deleteAccount := func(addr common.Address) {
		delete(addrs, addr)
		toDel := make([]storageKey, 0)
		for k := range storages {
			if k.addr == addr {
				toDel = append(toDel, k)
			}
		}
		for _, k := range toDel {
			delete(storages, k)
		}
	}

	memdb := rawdb.NewMemoryDatabase()
	tdb := state.NewDatabase(memdb)
	ethRoot := types.EmptyRootHash

	for i := 0; i < 10_000; i++ {
		tr, err := tdb.OpenTrie(ethRoot)
		require.NoError(t, err)
		mergeSet := trie.NewMergedNodeSet()

		var fwKeys, fwVals [][]byte

		switch {
		case i%20 == 19: // delete storage
			storageKey := chooseStorage()
			accHash := hashData(storageKey.addr[:])
			keyHash := hashData(storageKey.key[:])

			acc, err := tr.TryGetAccount(storageKey.addr)
			require.NoError(t, err)

			str, err := tdb.OpenStorageTrie(ethRoot, accHash, acc.Root)
			require.NoError(t, err)

			err = str.TryDelete(storageKey.key[:])
			require.NoError(t, err)
			deleteStorage(storageKey)

			strRoot, set := str.Commit(false)
			err = mergeSet.Merge(set)
			require.NoError(t, err)
			acc.Root = strRoot
			err = tr.TryUpdateAccount(storageKey.addr, acc)
			require.NoError(t, err)

			fwKeys = append(fwKeys, append(accHash[:], keyHash[:]...))
			fwVals = append(fwVals, []byte{})
		case i%100 == 99: // delete acc
			addr := chooseAddr()
			accHash := hashData(addr[:])

			err = tr.TryDeleteAccount(addr)
			require.NoError(t, err)
			deleteAccount(addr)

			fwKeys = append(fwKeys, accHash[:])
			fwVals = append(fwVals, []byte{})

		case i%4 == 0: // add acc
			addr := common.BytesToAddress(hashData(binary.BigEndian.AppendUint64(nil, uint64(i))).Bytes())
			accHash := hashData(addr[:])
			acc := &types.StateAccount{
				Nonce:    1,
				Balance:  new(big.Int).SetUint64(100),
				Root:     types.EmptyRootHash,
				CodeHash: types.EmptyCodeHash[:],
			}
			enc, err := rlp.EncodeToBytes(acc)
			require.NoError(t, err)

			err = tr.TryUpdateAccount(addr, acc)
			require.NoError(t, err)
			addrs[addr] = struct{}{}

			fwKeys = append(fwKeys, accHash[:])
			fwVals = append(fwVals, enc)
		case i%4 == 1: // update acc
			addr := chooseAddr()
			accHash := hashData(addr[:])
			acc, err := tr.TryGetAccount(addr)
			require.NoError(t, err)
			acc.Nonce++
			enc, err := rlp.EncodeToBytes(acc)
			require.NoError(t, err)

			err = tr.TryUpdateAccount(addr, acc)
			require.NoError(t, err)

			fwKeys = append(fwKeys, accHash[:])
			fwVals = append(fwVals, enc)
		case i%4 == 2: // add storage
			addr := chooseAddr()
			accHash := hashData(addr[:])
			key := hashData(binary.BigEndian.AppendUint64(nil, uint64(i)))
			keyHash := hashData(key[:])

			val := hashData(binary.BigEndian.AppendUint64(nil, uint64(i+1)))
			storageKey := storageKey{addr: addr, key: key}

			acc, err := tr.TryGetAccount(addr)
			require.NoError(t, err)

			str, err := tdb.OpenStorageTrie(ethRoot, accHash, acc.Root)
			require.NoError(t, err)

			err = str.TryUpdate(key[:], val[:])
			require.NoError(t, err)
			storages[storageKey] = struct{}{}

			strRoot, set := str.Commit(false)
			err = mergeSet.Merge(set)
			require.NoError(t, err)
			acc.Root = strRoot
			err = tr.TryUpdateAccount(addr, acc)
			require.NoError(t, err)

			fwKeys = append(fwKeys, append(accHash[:], keyHash[:]...))
			fwVals = append(fwVals, val[:])
		case i%4 == 3: // update storage
			storageKey := chooseStorage()
			accHash := hashData(storageKey.addr[:])
			keyHash := hashData(storageKey.key[:])

			val := hashData(binary.BigEndian.AppendUint64(nil, uint64(i+1)))

			acc, err := tr.TryGetAccount(storageKey.addr)
			require.NoError(t, err)

			str, err := tdb.OpenStorageTrie(ethRoot, accHash, acc.Root)
			require.NoError(t, err)

			err = str.TryUpdate(storageKey.key[:], val[:])
			require.NoError(t, err)

			strRoot, set := str.Commit(false)
			err = mergeSet.Merge(set)
			require.NoError(t, err)
			acc.Root = strRoot
			err = tr.TryUpdateAccount(storageKey.addr, acc)
			require.NoError(t, err)

			fwKeys = append(fwKeys, append(accHash[:], keyHash[:]...))
			fwVals = append(fwVals, val[:])
		}
		next, set := tr.Commit(true)
		err = mergeSet.Merge(set)
		require.NoError(t, err)

		err = tdb.TrieDB().Update(mergeSet)
		require.NoError(t, err)
		t.Logf("i: %d, next: %x", i, next)

		for i, k := range fwKeys {
			t.Logf("key: %x, val: %x", k, fwVals[i])
		}

		// update firewood db
		got, err := db.Update(fwKeys, fwVals)
		require.NoError(t, err)
		require.Equal(t, next[:], got)

		ethRoot = next
	}
}
