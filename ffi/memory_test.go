// Package firewood provides a Go wrapper around the [Firewood] database.
//
// [Firewood]: https://github.com/ava-labs/firewood
package firewood

import (
	"testing"

	"github.com/stretchr/testify/assert"
)

// This checks all inputs for `extractErrorThenFree`
func TestExtractError(t *testing.T) {
	valueTests := []struct {
		name          string
		expectedError string
		value         *testValue
	}{
		{
			name:          "nil",
			expectedError: errNilValue.Error(),
			value:         nil,
		},
		{
			name:          "length zero",
			expectedError: "",
			value:         newCValueLen(0),
		},
		{
			name:          "length nonzero",
			expectedError: errBadValue.Error(),
			value:         newCValueLen(1),
		},
		{
			name:          "data length nonzero",
			expectedError: errBadValue.Error(),
			value:         newCValueData(11, "test bytes"), // count null pointer created by C
		},
		{
			name:          "data zero",
			expectedError: "test bytes",
			value:         newCValueData(0, "test bytes"),
		},
	}

	for _, v := range valueTests {
		t.Run(v.name, func(t *testing.T) {
			err := extractErrorThenFree(v.value)
			if err == nil {
				assert.Equal(t, v.expectedError, "")
			} else {
				assert.Equal(t, v.expectedError, err.Error())
			}
		})
	}
}

func TestExtractIntError(t *testing.T) {
	valueTests := []struct {
		name          string
		expectedValue uint32
		expectedError string
		value         *testValue
	}{
		{
			name:          "nil",
			expectedValue: 0,
			expectedError: errNilValue.Error(),
			value:         nil,
		},
		{
			name:          "length zero",
			expectedValue: 0,
			expectedError: errBadValue.Error(),
			value:         newCValueLen(0),
		},
		{
			name:          "length nonzero",
			expectedValue: 1,
			expectedError: "",
			value:         newCValueLen(1),
		},
		{
			name:          "data length nonzero",
			expectedValue: 0,
			expectedError: errBadValue.Error(),
			value:         newCValueData(11, "test bytes"), // count null pointer created by C
		},
		{
			name:          "data zero",
			expectedValue: 0,
			expectedError: "test bytes",
			value:         newCValueData(0, "test bytes"),
		},
	}

	for _, v := range valueTests {
		t.Run(v.name, func(t *testing.T) {
			id, err := extractIdThenFree(v.value)
			assert.Equal(t, v.expectedValue, id)
			if err == nil {
				assert.Equal(t, v.expectedError, "")
			} else {
				assert.Equal(t, v.expectedError, err.Error())
			}
		})
	}
}

func TestExtractBytesError(t *testing.T) {
	valueTests := []struct {
		name          string
		expectedValue []byte
		expectedError string
		value         *testValue
	}{
		{
			name:          "nil",
			expectedValue: nil,
			expectedError: errNilValue.Error(),
			value:         nil,
		},
		{
			name:          "length zero",
			expectedValue: nil,
			expectedError: errBadValue.Error(),
			value:         newCValueLen(0),
		},
		{
			name:          "length nonzero",
			expectedValue: nil,
			expectedError: errBadValue.Error(),
			value:         newCValueLen(1),
		},
		{
			name:          "data length nonzero",
			expectedValue: []byte("test bytes\x00"), // count null pointer created by C
			expectedError: "",
			value:         newCValueData(11, "test bytes"), // count null pointer created by C
		},
		{
			name:          "data zero",
			expectedValue: nil,
			expectedError: "test bytes",
			value:         newCValueData(0, "test bytes"),
		},
	}
	for _, v := range valueTests {
		t.Run(v.name, func(t *testing.T) {
			bytes, err := extractBytesThenFree(v.value)
			assert.Equal(t, v.expectedValue, bytes)
			if err == nil {
				assert.Equal(t, v.expectedError, "")
			} else {
				assert.Equal(t, v.expectedError, err.Error())
			}
		})
	}
}
