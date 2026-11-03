package wire

import (
	"encoding/json"
	"reflect"
	"strings"
	"testing"
)

func TestBlobReferenceDecoder(t *testing.T) {
	a, b, c := strings.Repeat("a", 64), strings.Repeat("b", 64), strings.Repeat("c", 64)
	vectors := []struct {
		name, raw string
		want      []DigestReference
	}{
		{"root_and_derivative_order", `{"digest":"` + a + `","image":{"digest":"` + c + `"},"derivatives":[{"digest":"` + b + `"}],"counter":9007199254740993}`, []DigestReference{{a}, {b}, {c}}},
		{"raw_body_and_duplicates", `{"raw_body_digest":"` + a + `","images":[{"digest":"` + a + `"},{"digest":"` + b + `"}]}`, []DigestReference{{a}, {a}, {b}}},
		{"invalid_digest_shapes", `{"digest":"` + strings.Repeat("A", 64) + `","items":[{"digest":42},{"digest":"short"}]}`, []DigestReference{}},
		{"invalid_document", `{"digest":"` + a + `"} trailing`, nil},
		{"unrepresentable_number", `{"digest":"` + a + `","number":1e10000}`, nil},
	}
	for _, v := range vectors {
		t.Run(v.name, func(t *testing.T) {
			if got := BlobReferences(json.RawMessage(v.raw)); !reflect.DeepEqual(got, v.want) {
				t.Fatalf("reference codec mismatch: got %v want %v", got, v.want)
			}
		})
	}
}
