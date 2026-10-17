package contracts

import (
	"encoding/hex"
	"encoding/json"
	"time"
)

type BlobReference struct {
	Digest string `json:"digest"`
	Size   int64  `json:"size"`
	State  string `json:"state"`
	Handle string `json:"-"`
}
type ReferenceSet map[string][]BlobReference

func ValidDigest(d string) bool {
	b, err := hex.DecodeString(d)
	return err == nil && len(b) == 32 && hex.EncodeToString(b) == d
}
func BlobReferences(data json.RawMessage) []BlobReference {
	var value any
	if json.Unmarshal(data, &value) != nil {
		return nil
	}
	out := []BlobReference{}
	var walk func(any, int64)
	walk = func(v any, size int64) {
		switch x := v.(type) {
		case map[string]any:
			if n, ok := x["size"].(float64); ok {
				size = int64(n)
			}
			if d, ok := x["digest"].(string); ok && ValidDigest(d) {
				state, _ := x["state"].(string)
				handle, _ := x["handle"].(string)
				out = append(out, BlobReference{Digest: d, Size: size, State: state, Handle: handle})
			}
			if d, ok := x["raw_body_digest"].(string); ok && ValidDigest(d) {
				out = append(out, BlobReference{Digest: d})
			}
			for _, child := range x {
				walk(child, size)
			}
		case []any:
			for _, child := range x {
				walk(child, size)
			}
		}
	}
	walk(value, 0)
	return out
}

type BlobEntry struct {
	ResumedBytes int64 `json:"resumed_bytes"`
	Corruptions  int   `json:"corruptions"`

	State          string    `json:"state"`
	Size           int64     `json:"size"`
	Received       int64     `json:"received"`
	UnreferencedAt time.Time `json:"unreferenced_at,omitempty"`
}
type BlobProgress struct {
	Entries      map[string]BlobEntry `json:"entries"`
	Bytes        int64                `json:"bytes"`
	Backlog      int                  `json:"backlog"`
	ResumedBytes int64                `json:"resumed_bytes"`
	Corruptions  int                  `json:"corruptions"`
}
