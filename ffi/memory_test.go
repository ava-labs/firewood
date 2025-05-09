// Package firewood provides a Go wrapper around the [Firewood] database.
//
// [Firewood]: https://github.com/ava-labs/firewood
package firewood

import (
	"testing"

	"github.com/stretchr/testify/assert"
)

func TestExtractValue(t *testing.T) {
	valueTests := []struct {
		name          string
		expectedInt   uint32
		expectedBytes []byte
		expectedError string
		value         *testValue
	}{
		{
			name:          "nil",
			expectedInt:   0,
			expectedBytes: nil,
			expectedError: errNilValue.Error(),
			value:         nil,
		},
		{
			name:          "length zero",
			expectedInt:   0,
			expectedBytes: nil,
			expectedError: "",
			value:         newCValueLen(0),
		},
		{
			name:          "length nonzero",
			expectedInt:   1,
			expectedBytes: nil,
			expectedError: "",
			value:         newCValueLen(1),
		},
		{
			name:          "data length nonzero",
			expectedInt:   0,
			expectedBytes: []byte("test bytes\x00"), // count null pointer created by C
			expectedError: "",
			value:         newCValueData(11, "test bytes"), // count null pointer created by C
		},
		{
			name:          "data zero",
			expectedInt:   0,
			expectedBytes: nil,
			expectedError: "test bytes",
			value:         newCValueData(0, "test bytes"),
		},
	}
	for _, v := range valueTests {
		t.Run(v.name, func(t *testing.T) {
			id, bytes, err := extractValueThenFree(v.value)
			assert.Equal(t, v.expectedInt, id)
			assert.Equal(t, v.expectedBytes, bytes)
			if err == nil {
				assert.Equal(t, v.expectedError, "")
			} else {
				assert.Equal(t, v.expectedError, err.Error())
			}
		})
	}
}
