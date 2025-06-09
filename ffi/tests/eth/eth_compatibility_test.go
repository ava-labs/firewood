package tests

import (
	"encoding/binary"
	"math/rand"
	"path"
	"slices"
	"testing"

	firewood "github.com/ava-labs/firewood-go/ffi"
	"github.com/ava-labs/libevm/common"
	"github.com/ava-labs/libevm/core/rawdb"
	"github.com/ava-labs/libevm/core/state"
	"github.com/ava-labs/libevm/core/types"
	"github.com/ava-labs/libevm/crypto"
	"github.com/ava-labs/libevm/rlp"
	"github.com/ava-labs/libevm/trie/trienode"
	"github.com/ava-labs/libevm/triedb"
	"github.com/holiman/uint256"
	"github.com/stretchr/testify/require"
)

const (
	commit byte = iota
	createAccount
	updateAccount
	deleteAccount
	addStorage
	updateStorage
	deleteStorage
	maxStep
)

var (
	stepMap = map[byte]string{
		commit:        "commit",
		createAccount: "createAccount",
		updateAccount: "updateAccount",
		deleteAccount: "deleteAccount",
		addStorage:    "addStorage",
		updateStorage: "updateStorage",
		deleteStorage: "deleteStorage",
	}
)

type storageKey struct {
	addr common.Address
	key  common.Hash
}

type tree struct {
	fwdDB       *firewood.Database
	accountTrie state.Trie
	ethDatabase state.Database

	lastRoot common.Hash
	require  *require.Assertions

	// current state
	currentAddrs               []common.Address
	currentStorage             map[common.Address]map[common.Hash]common.Hash
	currentStorageInputIndices map[common.Address]uint64
	inputCounter               uint64

	pendingMergeSet *trienode.MergedNodeSet
	pendingFwdKeys  [][]byte
	pendingFwdVals  [][]byte
}

func newTestTree(t *testing.T) *tree {
	r := require.New(t)

	file := path.Join(t.TempDir(), "test.db")
	cfg := firewood.DefaultConfig()
	cfg.Create = true
	cfg.MetricsPort = 0
	db, err := firewood.New(file, cfg)
	r.NoError(err)

	tdb := state.NewDatabaseWithConfig(rawdb.NewMemoryDatabase(), triedb.HashDefaults)
	ethRoot := types.EmptyRootHash
	tr, err := tdb.OpenTrie(ethRoot)
	r.NoError(err)
	t.Cleanup(func() {
		r.NoError(db.Close())
	})

	return &tree{
		fwdDB:                      db,
		accountTrie:                tr,
		ethDatabase:                tdb,
		currentStorage:             make(map[common.Address]map[common.Hash]common.Hash),
		currentStorageInputIndices: make(map[common.Address]uint64),
		require:                    r,
		pendingMergeSet:            trienode.NewMergedNodeSet(),
	}
}

func (tr *tree) commit() {
	updatedRoot, set, err := tr.accountTrie.Commit(true)
	tr.require.NoError(err)
	if set != nil {
		tr.require.NoError(tr.pendingMergeSet.Merge(set))
	}

	tr.require.NoError(tr.ethDatabase.TrieDB().Update(updatedRoot, tr.lastRoot, 0, tr.pendingMergeSet, nil))
	tr.lastRoot = updatedRoot

	fwdRoot, err := tr.fwdDB.Update(tr.pendingFwdKeys, tr.pendingFwdVals)
	tr.require.NoError(err)
	tr.require.Equal(fwdRoot, updatedRoot[:])

	tr.pendingFwdKeys = nil
	tr.pendingFwdVals = nil

	tr.pendingMergeSet = trienode.NewMergedNodeSet()
	tr.accountTrie, err = tr.ethDatabase.OpenTrie(tr.lastRoot)
	tr.require.NoError(err)
}

func (tr *tree) createAccount() {
	addr := common.BytesToAddress(crypto.Keccak256Hash(binary.BigEndian.AppendUint64(nil, tr.inputCounter)).Bytes())
	tr.inputCounter++
	accHash := crypto.Keccak256Hash(addr[:])
	acc := &types.StateAccount{
		Nonce:    1,
		Balance:  uint256.NewInt(100),
		Root:     types.EmptyRootHash,
		CodeHash: types.EmptyCodeHash[:],
	}
	accountRLP, err := rlp.EncodeToBytes(acc)
	tr.require.NoError(err)

	err = tr.accountTrie.UpdateAccount(addr, acc)
	tr.require.NoError(err)
	tr.currentAddrs = append(tr.currentAddrs, addr)

	tr.pendingFwdKeys = append(tr.pendingFwdKeys, accHash[:])
	tr.pendingFwdVals = append(tr.pendingFwdVals, accountRLP)
}

func (tr *tree) selectAccount(addrIndex int) (common.Address, common.Hash) {
	addr := tr.currentAddrs[addrIndex]
	return addr, crypto.Keccak256Hash(addr[:])
}

func (tr *tree) updateAccount(addrIndex int) {
	addr, accHash := tr.selectAccount(addrIndex)
	acc, err := tr.accountTrie.GetAccount(addr)
	tr.require.NoError(err)
	acc.Nonce++
	accountRLP, err := rlp.EncodeToBytes(acc)
	tr.require.NoError(err)

	err = tr.accountTrie.UpdateAccount(addr, acc)
	tr.require.NoError(err)

	tr.pendingFwdKeys = append(tr.pendingFwdKeys, accHash[:])
	tr.pendingFwdVals = append(tr.pendingFwdVals, accountRLP)
}

func (tr *tree) deleteAccount(accountIndex int) {
	deleteAddr, accHash := tr.selectAccount(accountIndex)

	tr.require.NoError(tr.accountTrie.DeleteAccount(deleteAddr))
	tr.currentAddrs = slices.DeleteFunc(tr.currentAddrs, func(addr common.Address) bool {
		return deleteAddr == addr
	})
	delete(tr.currentStorage, deleteAddr)

	tr.pendingFwdKeys = append(tr.pendingFwdKeys, accHash[:])
	tr.pendingFwdVals = append(tr.pendingFwdVals, []byte{})
}

func (tr *tree) addStorage(accountIndex int) {
	addr, accHash := tr.selectAccount(accountIndex)
	// Derive new storage key-value pair from storage index
	storageIndex := tr.currentStorageInputIndices[addr]
	key := crypto.Keccak256Hash(binary.BigEndian.AppendUint64(nil, storageIndex))
	keyHash := crypto.Keccak256Hash(key[:])
	val := crypto.Keccak256Hash(keyHash[:])
	tr.currentStorageInputIndices[addr]++

	acc, err := tr.accountTrie.GetAccount(addr)
	tr.require.NoError(err)

	str, err := tr.ethDatabase.OpenStorageTrie(tr.lastRoot, addr, acc.Root, tr.accountTrie)
	tr.require.NoError(err)

	err = str.UpdateStorage(addr, key[:], val[:])
	tr.require.NoError(err)

	accountStateRoot, set, err := str.Commit(false)
	tr.require.NoError(err)
	if set != nil {
		tr.require.NoError(tr.pendingMergeSet.Merge(set))
	}
	acc.Root = accountStateRoot
	tr.require.NoError(tr.accountTrie.UpdateAccount(addr, acc))

	// Update storage key-value pair in firewood
	tr.pendingFwdKeys = append(tr.pendingFwdKeys, append(accHash[:], keyHash[:]...))
	tr.pendingFwdVals = append(tr.pendingFwdVals, val[:])

	// Update account in firewood
	updatedAccountRLP, err := rlp.EncodeToBytes(acc)
	tr.require.NoError(err)
	tr.pendingFwdKeys = append(tr.pendingFwdKeys, accHash[:])
	tr.pendingFwdVals = append(tr.pendingFwdVals, updatedAccountRLP)

	storageMap, ok := tr.currentStorage[addr]
	if !ok {
		storageMap = make(map[common.Hash]common.Hash)
		tr.currentStorage[addr] = storageMap
	}
	storageMap[keyHash] = val
}

func (tr *tree) updateStorage(accountIndex int, storageIndexInput uint64) {
	addr, accHash := tr.selectAccount(accountIndex)
	storageMap, ok := tr.currentStorage[addr]
	if !ok {
		storageMap = make(map[common.Hash]common.Hash)
		tr.currentStorage[addr] = storageMap
	}
	storageIndex := tr.currentStorageInputIndices[addr]
	storageIndex %= storageIndexInput

	storageKey := crypto.Keccak256Hash(binary.BigEndian.AppendUint64(nil, storageIndex))
	storageKeyHash := crypto.Keccak256Hash(storageKey[:])
	updatedValInput := binary.BigEndian.AppendUint64(storageKeyHash[:], tr.inputCounter)
	updatedVal := crypto.Keccak256Hash(updatedValInput[:])
	tr.inputCounter++

	acc, err := tr.accountTrie.GetAccount(addr)
	tr.require.NoError(err)

	str, err := tr.ethDatabase.OpenStorageTrie(tr.lastRoot, addr, acc.Root, tr.accountTrie)
	tr.require.NoError(err)

	tr.require.NoError(str.UpdateStorage(addr, storageKey[:], updatedVal[:]))

	strRoot, set, err := str.Commit(false)
	tr.require.NoError(err)
	if set != nil {
		tr.require.NoError(tr.pendingMergeSet.Merge(set))
	}
	acc.Root = strRoot
	tr.require.NoError(tr.accountTrie.UpdateAccount(addr, acc))

	tr.pendingFwdKeys = append(tr.pendingFwdKeys, append(accHash[:], storageKeyHash[:]...))
	updatedValRLP, err := rlp.EncodeToBytes(updatedVal[:])
	tr.require.NoError(err)
	tr.pendingFwdVals = append(tr.pendingFwdVals, updatedValRLP[:])

	updatedAccountRLP, err := rlp.EncodeToBytes(acc)
	tr.require.NoError(err)
	tr.pendingFwdKeys = append(tr.pendingFwdKeys, accHash[:])
	tr.pendingFwdVals = append(tr.pendingFwdVals, updatedAccountRLP)
}

func (tr *tree) deleteStorage(accountIndex int, storageIndexInput uint64) {
	addr, accHash := tr.selectAccount(accountIndex)
	storageMap, ok := tr.currentStorage[addr]
	if !ok {
		storageMap = make(map[common.Hash]common.Hash)
		tr.currentStorage[addr] = storageMap
	}
	storageIndex := tr.currentStorageInputIndices[addr]
	storageIndex %= storageIndexInput
	storageKey := crypto.Keccak256Hash(binary.BigEndian.AppendUint64(nil, storageIndex))
	storageKeyHash := crypto.Keccak256Hash(storageKey[:])

	acc, err := tr.accountTrie.GetAccount(addr)
	tr.require.NoError(err)

	str, err := tr.ethDatabase.OpenStorageTrie(tr.lastRoot, addr, acc.Root, tr.accountTrie)
	tr.require.NoError(err)

	tr.require.NoError(str.DeleteStorage(addr, storageKey[:]))

	strRoot, set, err := str.Commit(false)
	tr.require.NoError(err)
	if set != nil {
		tr.require.NoError(tr.pendingMergeSet.Merge(set))
	}
	acc.Root = strRoot
	tr.require.NoError(tr.accountTrie.UpdateAccount(addr, acc))

	tr.pendingFwdKeys = append(tr.pendingFwdKeys, append(accHash[:], storageKeyHash[:]...))
	tr.pendingFwdVals = append(tr.pendingFwdVals, []byte{})

	updatedAccountRLP, err := rlp.EncodeToBytes(acc)
	tr.require.NoError(err)
	tr.pendingFwdKeys = append(tr.pendingFwdKeys, accHash[:])
	tr.pendingFwdVals = append(tr.pendingFwdVals, updatedAccountRLP)
}

func FuzzTree(f *testing.F) {
	f.Fuzz(func(t *testing.T, randSeed int64, byteSteps []byte) {
		tr := newTestTree(t)
		rand := rand.New(rand.NewSource(randSeed))

		for _ = range 10 {
			tr.createAccount()
		}
		tr.commit()

		const maxSteps = 1000
		if len(byteSteps) > maxSteps {
			byteSteps = byteSteps[:maxSteps]
		}

		for _, step := range byteSteps {
			step = step % maxStep
			t.Log(stepMap[step])
			switch step {
			case commit:
				tr.commit()
			case createAccount:
				tr.createAccount()
			case updateAccount:
				if len(tr.currentAddrs) > 0 {
					tr.updateAccount(rand.Intn(len(tr.currentAddrs)))
				}
			case deleteAccount:
				if len(tr.currentAddrs) > 0 {
					tr.deleteAccount(rand.Intn(len(tr.currentAddrs)))
				}
			case addStorage:
				if len(tr.currentAddrs) > 0 {
					tr.addStorage(rand.Intn(len(tr.currentAddrs)))
				}
			case updateStorage:
				if len(tr.currentAddrs) > 0 {
					tr.updateStorage(rand.Intn(len(tr.currentAddrs)), rand.Uint64())
				}
			case deleteStorage:
				if len(tr.currentAddrs) > 0 {
					tr.deleteStorage(rand.Intn(len(tr.currentAddrs)), rand.Uint64())
				}
			default:
				t.Fatalf("unknown step: %d", step)
			}
		}
	})
}

func TestInsert(t *testing.T) {
	t.Skip()
	file := path.Join(t.TempDir(), "test.db")
	cfg := firewood.DefaultConfig()
	cfg.Create = true
	db, err := firewood.New(file, cfg)
	require.NoError(t, err)
	defer db.Close()

	rand := rand.New(rand.NewSource(0))

	addrs := make([]common.Address, 0)
	storages := make([]storageKey, 0)

	chooseAddr := func() common.Address {
		return addrs[rand.Intn(len(addrs))] //nolint:gosec
	}

	chooseStorage := func() storageKey {
		return storages[rand.Intn(len(storages))] //nolint:gosec
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
	tdb := state.NewDatabaseWithConfig(memdb, triedb.HashDefaults)
	ethRoot := types.EmptyRootHash

	for i := range uint64(10_000) {
		tr, err := tdb.OpenTrie(ethRoot)
		require.NoError(t, err)
		mergeSet := trienode.NewMergedNodeSet()

		var fwKeys, fwVals [][]byte

		switch {
		case i%100 == 99: // delete acc
			addr := chooseAddr()
			accHash := crypto.Keccak256Hash(addr[:])

			err = tr.DeleteAccount(addr)
			require.NoError(t, err)
			deleteAccount(addr)

			fwKeys = append(fwKeys, accHash[:])
			fwVals = append(fwVals, []byte{})
		case i%10 == 9: // delete storage
			storageKey := chooseStorage()
			accHash := crypto.Keccak256Hash(storageKey.addr[:])
			keyHash := crypto.Keccak256Hash(storageKey.key[:])

			acc, err := tr.GetAccount(storageKey.addr)
			require.NoError(t, err)

			str, err := tdb.OpenStorageTrie(ethRoot, storageKey.addr, acc.Root, tr)
			require.NoError(t, err)

			err = str.DeleteStorage(storageKey.addr, storageKey.key[:])
			require.NoError(t, err)
			deleteStorage(storageKey)

			strRoot, set, err := str.Commit(false)
			require.NoError(t, err)
			err = mergeSet.Merge(set)
			require.NoError(t, err)
			acc.Root = strRoot
			err = tr.UpdateAccount(storageKey.addr, acc)
			require.NoError(t, err)

			fwKeys = append(fwKeys, append(accHash[:], keyHash[:]...))
			fwVals = append(fwVals, []byte{})

			// We must also update the account (not for hash, but to be accurate)
			fwKeys = append(fwKeys, accHash[:])
			encodedVal, err := rlp.EncodeToBytes(acc)
			require.NoError(t, err)
			fwVals = append(fwVals, encodedVal)
		case i%4 == 0: // add acc
			addr := common.BytesToAddress(crypto.Keccak256Hash(binary.BigEndian.AppendUint64(nil, i)).Bytes())
			accHash := crypto.Keccak256Hash(addr[:])
			acc := &types.StateAccount{
				Nonce:    1,
				Balance:  uint256.NewInt(100),
				Root:     types.EmptyRootHash,
				CodeHash: types.EmptyCodeHash[:],
			}
			enc, err := rlp.EncodeToBytes(acc)
			require.NoError(t, err)

			err = tr.UpdateAccount(addr, acc)
			require.NoError(t, err)
			addrs = append(addrs, addr)

			fwKeys = append(fwKeys, accHash[:])
			fwVals = append(fwVals, enc)
		case i%4 == 1: // update acc
			addr := chooseAddr()
			accHash := crypto.Keccak256Hash(addr[:])
			acc, err := tr.GetAccount(addr)
			require.NoError(t, err)
			acc.Nonce++
			enc, err := rlp.EncodeToBytes(acc)
			require.NoError(t, err)

			err = tr.UpdateAccount(addr, acc)
			require.NoError(t, err)

			fwKeys = append(fwKeys, accHash[:])
			fwVals = append(fwVals, enc)
		case i%4 == 2: // add storage
			addr := chooseAddr()
			accHash := crypto.Keccak256Hash(addr[:])
			key := crypto.Keccak256Hash(binary.BigEndian.AppendUint64(nil, i))
			keyHash := crypto.Keccak256Hash(key[:])

			val := crypto.Keccak256Hash(binary.BigEndian.AppendUint64(nil, i+1))
			storageKey := storageKey{addr: addr, key: key}

			acc, err := tr.GetAccount(addr)
			require.NoError(t, err)

			str, err := tdb.OpenStorageTrie(ethRoot, addr, acc.Root, tr)
			require.NoError(t, err)

			err = str.UpdateStorage(addr, key[:], val[:])
			require.NoError(t, err)
			storages = append(storages, storageKey)

			strRoot, set, err := str.Commit(false)
			require.NoError(t, err)
			err = mergeSet.Merge(set)
			require.NoError(t, err)
			acc.Root = strRoot
			err = tr.UpdateAccount(addr, acc)
			require.NoError(t, err)

			fwKeys = append(fwKeys, append(accHash[:], keyHash[:]...))
			// UpdateStorage automatically encodes the value to rlp,
			// so we need to encode prior to sending to firewood
			encodedVal, err := rlp.EncodeToBytes(val[:])
			require.NoError(t, err)
			fwVals = append(fwVals, encodedVal)

			// We must also update the account (not for hash, but to be accurate)
			fwKeys = append(fwKeys, accHash[:])
			encodedVal, err = rlp.EncodeToBytes(acc)
			require.NoError(t, err)
			fwVals = append(fwVals, encodedVal)
		case i%4 == 3: // update storage
			storageKey := chooseStorage()
			accHash := crypto.Keccak256Hash(storageKey.addr[:])
			keyHash := crypto.Keccak256Hash(storageKey.key[:])

			val := crypto.Keccak256Hash(binary.BigEndian.AppendUint64(nil, i+1))

			acc, err := tr.GetAccount(storageKey.addr)
			require.NoError(t, err)

			str, err := tdb.OpenStorageTrie(ethRoot, storageKey.addr, acc.Root, tr)
			require.NoError(t, err)

			err = str.UpdateStorage(storageKey.addr, storageKey.key[:], val[:])
			require.NoError(t, err)

			strRoot, set, err := str.Commit(false)
			require.NoError(t, err)
			err = mergeSet.Merge(set)
			require.NoError(t, err)
			acc.Root = strRoot
			err = tr.UpdateAccount(storageKey.addr, acc)
			require.NoError(t, err)

			fwKeys = append(fwKeys, append(accHash[:], keyHash[:]...))
			// UpdateStorage automatically encodes the value to rlp,
			// so we need to encode prior to sending to firewood
			encodedVal, err := rlp.EncodeToBytes(val[:])
			require.NoError(t, err)
			fwVals = append(fwVals, encodedVal)

			// We must also update the account (not for hash, but to be accurate)
			fwKeys = append(fwKeys, accHash[:])
			encodedVal, err = rlp.EncodeToBytes(acc)
			require.NoError(t, err)
			fwVals = append(fwVals, encodedVal)
		}
		next, set, err := tr.Commit(true)
		require.NoError(t, err)
		err = mergeSet.Merge(set)
		require.NoError(t, err)

		err = tdb.TrieDB().Update(next, ethRoot, i, mergeSet, nil)
		require.NoError(t, err)

		// update firewood db
		got, err := db.Update(fwKeys, fwVals)
		require.NoError(t, err)
		require.Equal(t, next[:], got)

		ethRoot = next
	}
}
