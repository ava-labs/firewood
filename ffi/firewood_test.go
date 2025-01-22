package firewood

import (
	"crypto/sha256"
	"encoding/binary"
	"os"
	"strconv"
	"testing"
)

func TestInsert(t *testing.T) {
	var f Firewood = CreateDatabase("test.db")
	defer os.Remove("test.db")
	defer f.Close()
	f.Batch([]KeyValue{
		{[]byte("abc"), []byte("def")},
	})

	value, _ := f.Get([]byte("abc"))
	if string(value) != "def" {
		t.Errorf("expected def, got %s", value)
	}
}

func TestInsert100(t *testing.T) {
	var f Firewood = CreateDatabase("test.db")
	defer os.Remove("test.db")
	defer f.Close()
	ops := make([]KeyValue, 100)
	for i := 0; i < 100; i++ {
		ops[i] = KeyValue{[]byte("key" + strconv.Itoa(i)), []byte("value" + strconv.Itoa(i))}
	}
	f.Batch(ops)

	for i := 0; i < 100; i++ {
		value, err := f.Get([]byte("key" + strconv.Itoa(i)))
		if err != nil {
			t.FailNow()
		}
		if string(value) != "value"+strconv.Itoa(i) {
			t.Errorf("expected value%d, got %s", i, value)
		}
	}

	hash := f.Root()
	if len(hash) != 32 {
		t.Errorf("expected 32 bytes, got %d", len(hash))
	}

	// we know the hash starts with 0xf8
	if hash[0] != 0xf8 {
		t.Errorf("expected 0xf8, got %x", hash[0])
	}

	delete_ops := make([]KeyValue, 1)
	ops[0] = KeyValue{[]byte(""), []byte("")}
	f.Batch(delete_ops)
}

func TestRangeDelete(t *testing.T) {
	const N = 100
	var f Firewood = CreateDatabase("test.db")
	defer os.Remove("test.db")
	defer f.Close()
	ops := make([]KeyValue, N)
	for i := 0; i < N; i++ {
		ops[i] = KeyValue{[]byte("key" + strconv.Itoa(i)), []byte("value" + strconv.Itoa(i))}
	}
	f.Batch(ops)

	// delete all keys that start with "key"
	delete_ops := make([]KeyValue, 1)
	delete_ops[0] = KeyValue{[]byte("key1"), []byte("")}
	f.Batch(delete_ops)

	for i := 0; i < N; i++ {
		keystring := "key" + strconv.Itoa(i)
		value, err := f.Get([]byte(keystring))
		if err != nil {
			t.FailNow()
		}
		if (value != nil) == (keystring[3] == '1') {
			t.Errorf("incorrect response for %s %s %x", keystring, value, keystring[3])
		}
	}
}

func TestInvariants(t *testing.T) {
	var f Firewood = CreateDatabase("test.db")
	defer os.Remove("test.db")
	defer f.Close()

	// validate that the root of an empty trie is all zeroes
	empty_root := f.Root()
	if len(empty_root) != 32 {
		t.Errorf("expected 32 bytes, got %d", len(empty_root))
	}
	empty_array := [32]byte(empty_root)
	if empty_array != [32]byte{} {
		t.Errorf("expected empty root, got %x", empty_root)
	}

	// validate that get returns nil, nil for non-existent key
	val, err := f.Get([]byte("non-existent"))
	if val != nil || err != nil {
		t.Errorf("expected nil, nil, got %v, %v", val, err)
	}
}

var livesForever Firewood

func BenchmarkLotsOfBatches(b *testing.B) {
	var f Firewood = CreateDatabase("test.db")

	h := sha256.New()
	buf := make([]byte, 32)

	get := func(prefix string, i int, out []byte) []byte {
		copy(buf, prefix)
		binary.BigEndian.PutUint64(buf[len(prefix):], uint64(i))
		out = h.Sum(out)
		h.Reset()
		return out
	}

	batchSize := 10_000
	logEach := 10
	ks := make([][]byte, batchSize)
	vs := make([][]byte, batchSize)

	for i := 0; i < b.N; i++ {
		for j := 0; j < batchSize; j++ {
			ks[j] = get("key", i*batchSize+j, ks[j])
			vs[j] = get("value", i*batchSize+j, vs[j])
		}
		_, err := f.Update(ks, vs)
		if err != nil {
			b.Fatal(err)
		}
		if i%logEach == 0 {
			b.Logf("batch %d", i)
		}
	}

	livesForever = f
}
