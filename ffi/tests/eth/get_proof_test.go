// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package eth

import (
	"testing"

	"github.com/ava-labs/libevm/common"
	"github.com/ava-labs/libevm/core/rawdb"
	"github.com/ava-labs/libevm/core/state"
	"github.com/ava-labs/libevm/core/types"
	"github.com/ava-labs/libevm/crypto"
	"github.com/ava-labs/libevm/ethdb/memorydb"
	"github.com/ava-labs/libevm/rlp"
	"github.com/ava-labs/libevm/trie"
	"github.com/ava-labs/libevm/trie/trienode"
	"github.com/ava-labs/libevm/triedb"
	"github.com/holiman/uint256"
	"github.com/stretchr/testify/require"

	firewood "github.com/ava-labs/firewood-go-ethhash/ffi"
)

// slot is a single storage entry: the raw 32-byte slot key and its value.
type slot struct {
	key common.Hash
	val common.Hash
}

// accountSpec describes an account to materialize into the test state.
type accountSpec struct {
	addr  common.Address
	slots []slot
}

// builtState is the result of materializing accountSpecs into a firewood
// database and an equivalent go-ethereum trie.
type builtState struct {
	db   *firewood.Database
	root firewood.Hash
	// accountRLP maps account hash -> the RLP we stored for that account.
	accountRLP map[common.Hash][]byte
}

// buildState materializes specs into both a libevm state trie (to compute the
// canonical Ethereum storage roots and account RLPs) and a firewood database
// (flat keccak-prefixed keys). It asserts the two roots agree, so the firewood
// proofs can be checked against go-ethereum's verifier.
func buildState(t *testing.T, specs []accountSpec) builtState {
	t.Helper()
	r := require.New(t)

	db := newFirewoodDB(t)
	tdb := state.NewDatabaseWithConfig(rawdb.NewMemoryDatabase(), triedb.HashDefaults)
	accountTrie, err := tdb.OpenTrie(types.EmptyRootHash)
	r.NoError(err)

	merged := trienode.NewMergedNodeSet()
	var batch []firewood.BatchOp
	accountRLP := make(map[common.Hash][]byte)

	for _, spec := range specs {
		accHash := crypto.Keccak256Hash(spec.addr[:])
		storageRoot := types.EmptyRootHash

		if len(spec.slots) > 0 {
			storageTrie, err := tdb.OpenStorageTrie(types.EmptyRootHash, spec.addr, types.EmptyRootHash, accountTrie)
			r.NoError(err)

			for _, s := range spec.slots {
				r.NoError(storageTrie.UpdateStorage(spec.addr, s.key[:], s.val[:]))

				// firewood stores storage at keccak(addr) ++ keccak(slotKey),
				// with the RLP-encoded value (matching the trie's leaf value).
				slotHash := crypto.Keccak256Hash(s.key[:])
				fwdKey := append(append([]byte{}, accHash[:]...), slotHash[:]...)
				encodedVal, err := rlp.EncodeToBytes(s.val[:])
				r.NoError(err)
				batch = append(batch, firewood.Put(fwdKey, encodedVal))
			}

			root, set, err := storageTrie.Commit(false)
			r.NoError(err)
			if set != nil {
				r.NoError(merged.Merge(set))
			}
			storageRoot = root
		}

		acc := &types.StateAccount{
			Nonce:    1,
			Balance:  uint256.NewInt(100),
			Root:     storageRoot,
			CodeHash: types.EmptyCodeHash[:],
		}
		r.NoError(accountTrie.UpdateAccount(spec.addr, acc))

		encodedAcc, err := rlp.EncodeToBytes(acc)
		r.NoError(err)
		accountRLP[accHash] = encodedAcc
		batch = append(batch, firewood.Put(accHash[:], encodedAcc))
	}

	ethRoot, set, err := accountTrie.Commit(true)
	r.NoError(err)
	if set != nil {
		r.NoError(merged.Merge(set))
	}
	r.NoError(tdb.TrieDB().Update(ethRoot, types.EmptyRootHash, 0, merged, nil))

	fwdRoot, err := db.Update(batch)
	r.NoError(err)
	r.Equal(ethRoot, common.Hash(fwdRoot), "firewood root must match go-ethereum root")

	return builtState{db: db, root: fwdRoot, accountRLP: accountRLP}
}

// verifyProof loads the proof nodes into an in-memory keccak-keyed database and
// runs go-ethereum's trie.VerifyProof, returning the proven value (nil for a
// valid exclusion proof).
func verifyProof(t *testing.T, root common.Hash, key []byte, nodes [][]byte) []byte {
	t.Helper()
	proofDB := memorydb.New()
	for _, node := range nodes {
		require.NoError(t, proofDB.Put(crypto.Keccak256(node), node))
	}
	value, err := trie.VerifyProof(root, key, proofDB)
	require.NoError(t, err)
	return value
}

func TestEthGetProofAccountInclusionAndExclusion(t *testing.T) {
	r := require.New(t)

	present := common.HexToAddress("0x00000000000000000000000000000000000000a1")
	other := common.HexToAddress("0x00000000000000000000000000000000000000b2")
	absent := common.HexToAddress("0x00000000000000000000000000000000000000ff")

	built := buildState(t, []accountSpec{{addr: present}, {addr: other}})
	root := common.Hash(built.root)

	rev, err := built.db.Revision(built.root)
	r.NoError(err)
	defer func() { r.NoError(rev.Drop()) }()

	// Inclusion: the account proof verifies to the stored account RLP, and the
	// returned scalars match.
	accHash := crypto.Keccak256Hash(present[:])
	proof, err := rev.EthGetProof(accHash[:], nil)
	r.NoError(err)
	r.Empty(proof.StorageProof)

	value := verifyProof(t, root, accHash[:], proof.AccountProof)
	r.Equal(built.accountRLP[accHash], value)

	r.Equal(uint64(1), proof.Nonce)
	r.Equal(types.EmptyRootHash, common.Hash(proof.StorageHash))
	r.Equal(types.EmptyCodeHash, common.Hash(proof.CodeHash))

	// Exclusion: an absent account verifies to a nil value (valid proof of
	// absence) and reports the default account shape.
	absentHash := crypto.Keccak256Hash(absent[:])
	absentProof, err := rev.EthGetProof(absentHash[:], nil)
	r.NoError(err)
	r.Nil(verifyProof(t, root, absentHash[:], absentProof.AccountProof))
	r.Equal(uint64(0), absentProof.Nonce)
	r.Equal(types.EmptyRootHash, common.Hash(absentProof.StorageHash))
}

func TestEthGetProofStorageInclusionAndExclusion(t *testing.T) {
	r := require.New(t)

	addr := common.HexToAddress("0x00000000000000000000000000000000000000c3")
	slots := []slot{
		{key: common.HexToHash("0x01"), val: common.HexToHash("0x1111")},
		{key: common.HexToHash("0x02"), val: common.HexToHash("0x2222")},
		{key: common.HexToHash("0x03"), val: common.HexToHash("0x3333")},
	}
	absentSlotKey := common.HexToHash("0xdead")

	built := buildState(t, []accountSpec{{addr: addr, slots: slots}})
	root := common.Hash(built.root)

	rev, err := built.db.Revision(built.root)
	r.NoError(err)
	defer func() { r.NoError(rev.Drop()) }()

	accHash := crypto.Keccak256Hash(addr[:])

	// The storage trie keys are keccak(slotKey); request all present slots plus
	// one absent slot.
	slotHashes := make([][]byte, 0, len(slots)+1)
	for _, s := range slots {
		h := crypto.Keccak256Hash(s.key[:])
		slotHashes = append(slotHashes, h[:])
	}
	absentSlotHash := crypto.Keccak256Hash(absentSlotKey[:])
	slotHashes = append(slotHashes, absentSlotHash[:])

	proof, err := rev.EthGetProof(accHash[:], slotHashes)
	r.NoError(err)
	r.Len(proof.StorageProof, len(slots)+1)

	// StorageHash must equal the canonical storage trie root.
	storageHash := common.Hash(proof.StorageHash)
	r.NotEqual(types.EmptyRootHash, storageHash)

	// The account proof still verifies against the revision root.
	r.Equal(built.accountRLP[accHash], verifyProof(t, root, accHash[:], proof.AccountProof))

	// Present slots: storage proof verifies to the RLP-encoded value.
	for i, s := range slots {
		entry := proof.StorageProof[i]
		slotHash := crypto.Keccak256Hash(s.key[:])
		r.Equal(slotHash[:], entry.Key[:])

		expectedVal, err := rlp.EncodeToBytes(s.val[:])
		r.NoError(err)
		r.Equal(expectedVal, entry.Value)
		r.Equal(expectedVal, verifyProof(t, storageHash, slotHash[:], entry.Proof))
	}

	// Absent slot: nil value and a valid exclusion proof.
	absentEntry := proof.StorageProof[len(slots)]
	r.Equal(absentSlotHash[:], absentEntry.Key[:])
	r.Nil(absentEntry.Value)
	r.Nil(verifyProof(t, storageHash, absentSlotHash[:], absentEntry.Proof))
}
