package scenarios

import (
	"encoding/json"
	"math"
	"testing"
)

func TestNotesIntegerEqual(t *testing.T) {
	for _, test := range []struct {
		name        string
		left, right any
		equal       bool
	}{
		{"different representations", json.Number("3"), float64(3), true},
		{"different counts", json.Number("3"), float64(4), false},
		{"integer decimal", json.Number("3.0"), int64(3), true},
		{"integer exponent", json.Number("3e1"), int64(30), true},
		{"fractional truth", json.Number("3.5"), float64(3), false},
		{"fractional projection", json.Number("3"), float64(3.5), false},
		{"equal fractions rejected", json.Number("3.5"), float64(3.5), false},
		{"maximum integer", json.Number("9223372036854775807"), int64(math.MaxInt64), true},
		{"overflow truth", json.Number("9223372036854775808"), int64(math.MaxInt64), false},
		{"overflow float", json.Number("9223372036854775807"), math.Ldexp(1, 63), false},
		{"nan", json.Number("3"), math.NaN(), false},
		{"infinity", json.Number("3"), math.Inf(1), false},
		{"missing", nil, float64(0), false},
		{"string rejected", "3", float64(3), false},
	} {
		t.Run(test.name, func(t *testing.T) {
			if equal := notesIntegerEqual(test.left, test.right); equal != test.equal {
				t.Fatalf("equal = %v, want %v", equal, test.equal)
			}
		})
	}
}
