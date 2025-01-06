package firewood

import (
	"strconv"
	"testing"
)

func TestInsert(t *testing.T) {
	var f Firewood
	f.Batch([]KeyValue{
		{[]byte("abc"), []byte("def")},
	})

	value := f.Get([]byte("abc"))
	if string(value) != "def" {
		t.Errorf("expected def, got %s", value)
	}
}

func TestInsert100(t *testing.T) {
	var f Firewood
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
}
