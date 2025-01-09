package firewood

import (
	"strconv"
	"testing"
)

func TestInsert(t *testing.T) {
	var f Firewood = NewFirewood("test.db")
	f.Batch([]KeyValue{
		{[]byte("abc"), []byte("def")},
	})

	value := f.Get([]byte("abc"))
	if string(value) != "def" {
		t.Errorf("expected def, got %s", value)
	}
}

func TestInsert100(t *testing.T) {
	var f Firewood = NewFirewood("test.db")
	ops := make([]KeyValue, 100)
	for i := 0; i < 100; i++ {
		ops[i] = KeyValue{[]byte("key" + strconv.Itoa(i)), []byte("value" + strconv.Itoa(i))}
	}
	f.Batch(ops)

	for i := 0; i < 100; i++ {
		value := f.Get([]byte("key" + strconv.Itoa(i)))
		if string(value) != "value"+strconv.Itoa(i) {
			t.Errorf("expected value%d, got %s", i, value)
		}
	}

	hash := f.RootHash()
	if len(hash) != 32 {
		t.Errorf("expected 32 bytes, got %d", len(hash))
	}

	// we know the hash starts with 0xf8
	if hash[0] != 0xf8 {
		t.Errorf("expected 0xf8, got %x", hash[0])
	}
}
