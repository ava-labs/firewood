package tests

import (
	"encoding/binary"
	"math/rand"
	"path"
	"slices"
	"testing"

	firewood "github.com/ava-labs/firewood/ffi/v2"
	"github.com/ethereum/go-ethereum/common"
	"github.com/ethereum/go-ethereum/core/rawdb"
	"github.com/ethereum/go-ethereum/core/state"
	"github.com/ethereum/go-ethereum/core/types"
	"github.com/ethereum/go-ethereum/crypto"
	"github.com/ethereum/go-ethereum/rlp"
	"github.com/ethereum/go-ethereum/trie/trienode"
	"github.com/ethereum/go-ethereum/triedb"
	"github.com/holiman/uint256"
	"github.com/stretchr/testify/require"
)

func hashData(input []byte) common.Hash {
	return crypto.Keccak256Hash(input)
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

	rand.Seed(0)

	addrs := make([]common.Address, 0)
	storages := make([]storageKey, 0)

	chooseAddr := func() common.Address {
		return addrs[rand.Intn(len(addrs))]
	}

	chooseStorage := func() storageKey {
		return storages[rand.Intn(len(storages))]
	}

	deleteStorage := func(k storageKey) {
		storages = slices.DeleteFunc(storages, func(s storageKey) bool {
			return s == k
		})
	}

	deleteAccount := func(addr common.Address) {
		addrs = slices.DeleteFunc(addrs, func(a common.Address) bool {
			return a == addr
		})
		storages = slices.DeleteFunc(storages, func(s storageKey) bool {
			return s.addr == addr
		})
	}

	memdb := rawdb.NewMemoryDatabase()
	tdb := state.NewDatabase(triedb.NewDatabase(memdb, triedb.HashDefaults), nil)
	ethRoot := types.EmptyRootHash

	for i := range 10_000 {
		tr, err := tdb.OpenTrie(ethRoot)
		require.NoError(t, err)
		mergeSet := trienode.NewMergedNodeSet()

		var fwKeys, fwVals [][]byte

		switch {
		case i%100 == 99: // delete acc
			addr := chooseAddr()
			accHash := hashData(addr[:])

			err = tr.DeleteAccount(addr)
			require.NoError(t, err)
			deleteAccount(addr)

			fwKeys = append(fwKeys, accHash[:])
			fwVals = append(fwVals, []byte{})
		case i%10 == 9: // delete storage
			storageKey := chooseStorage()
			accHash := hashData(storageKey.addr[:])
			keyHash := hashData(storageKey.key[:])

			acc, err := tr.GetAccount(storageKey.addr)
			require.NoError(t, err)

			str, err := tdb.OpenStorageTrie(ethRoot, storageKey.addr, acc.Root, tr)
			require.NoError(t, err)

			err = str.DeleteStorage(storageKey.addr, storageKey.key[:])
			require.NoError(t, err)
			deleteStorage(storageKey)

			strRoot, set := str.Commit(false)
			err = mergeSet.Merge(set)
			require.NoError(t, err)
			acc.Root = strRoot
			err = tr.UpdateAccount(storageKey.addr, acc, 0)
			require.NoError(t, err)

			fwKeys = append(fwKeys, append(accHash[:], keyHash[:]...))
			fwVals = append(fwVals, []byte{})

		case i%4 == 0: // add acc
			addr := common.BytesToAddress(hashData(binary.BigEndian.AppendUint64(nil, uint64(i))).Bytes())
			accHash := hashData(addr[:])
			acc := &types.StateAccount{
				Nonce:    1,
				Balance:  uint256.NewInt(100),
				Root:     types.EmptyRootHash,
				CodeHash: types.EmptyCodeHash[:],
			}
			enc, err := rlp.EncodeToBytes(acc)
			require.NoError(t, err)

			err = tr.UpdateAccount(addr, acc, 0)
			require.NoError(t, err)
			addrs = append(addrs, addr)

			fwKeys = append(fwKeys, accHash[:])
			fwVals = append(fwVals, enc)
		case i%4 == 1: // update acc
			addr := chooseAddr()
			accHash := hashData(addr[:])
			acc, err := tr.GetAccount(addr)
			require.NoError(t, err)
			acc.Nonce++
			enc, err := rlp.EncodeToBytes(acc)
			require.NoError(t, err)

			err = tr.UpdateAccount(addr, acc, 0)
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

			acc, err := tr.GetAccount(addr)
			require.NoError(t, err)

			str, err := tdb.OpenStorageTrie(ethRoot, addr, acc.Root, tr)
			require.NoError(t, err)

			err = str.UpdateStorage(addr, key[:], val[:])
			require.NoError(t, err)
			storages = append(storages, storageKey)

			strRoot, set := str.Commit(false)
			err = mergeSet.Merge(set)
			require.NoError(t, err)
			acc.Root = strRoot
			err = tr.UpdateAccount(addr, acc, 0)
			require.NoError(t, err)

			fwKeys = append(fwKeys, append(accHash[:], keyHash[:]...))
			// UpdateStorage automatically encodes the value to rlp,
			// so we need to encode prior to sending to firewood
			encodedVal, err := rlp.EncodeToBytes(val[:])
			require.NoError(t, err)
			fwVals = append(fwVals, encodedVal)
		case i%4 == 3: // update storage
			storageKey := chooseStorage()
			accHash := hashData(storageKey.addr[:])
			keyHash := hashData(storageKey.key[:])

			val := hashData(binary.BigEndian.AppendUint64(nil, uint64(i+1)))

			acc, err := tr.GetAccount(storageKey.addr)
			require.NoError(t, err)

			str, err := tdb.OpenStorageTrie(ethRoot, storageKey.addr, acc.Root, tr)
			require.NoError(t, err)

			err = str.UpdateStorage(storageKey.addr, storageKey.key[:], val[:])
			require.NoError(t, err)

			strRoot, set := str.Commit(false)
			err = mergeSet.Merge(set)
			require.NoError(t, err)
			acc.Root = strRoot
			err = tr.UpdateAccount(storageKey.addr, acc, 0)
			require.NoError(t, err)

			fwKeys = append(fwKeys, append(accHash[:], keyHash[:]...))
			// UpdateStorage automatically encodes the value to rlp,
			// so we need to encode prior to sending to firewood
			encodedVal, err := rlp.EncodeToBytes(val[:])
			require.NoError(t, err)
			fwVals = append(fwVals, encodedVal)
		}
		next, set := tr.Commit(true)
		err = mergeSet.Merge(set)
		require.NoError(t, err)

		err = tdb.TrieDB().Update(next, ethRoot, uint64(i), mergeSet, nil)
		require.NoError(t, err)

		// update firewood db
		got, err := db.Update(fwKeys, fwVals)
		require.NoError(t, err)
		require.Equal(t, next[:], got)

		ethRoot = next
	}
}
