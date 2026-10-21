package contracts

import (
	"encoding/hex"
	"encoding/json"
	"sort"
	"time"
)

type BlobReference struct {
	AwaitingVariants bool   `json:"awaiting_variants,omitempty"`
	Variant          bool   `json:"variant,omitempty"`
	Digest           string `json:"digest"`
	Size             int64  `json:"size"`
	State            string `json:"state"`
	Handle           string `json:"-"`
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
	var walk func(any, int64, bool, bool)
	walk = func(v any, size int64, variant bool, awaiting bool) {
		switch x := v.(type) {
		case map[string]any:
			_, media := x["media"]
			_, derivatives := x["derivatives"]
			awaiting = awaiting || (media && !derivatives)
			if n, ok := x["size"].(float64); ok {
				size = int64(n)
			}
			if d, ok := x["digest"].(string); ok && ValidDigest(d) {
				state, _ := x["state"].(string)
				handle, _ := x["handle"].(string)
				out = append(out, BlobReference{Digest: d, Size: size, State: state, Handle: handle, Variant: variant, AwaitingVariants: awaiting && !variant})
			}
			if d, ok := x["raw_body_digest"].(string); ok && ValidDigest(d) {
				out = append(out, BlobReference{Digest: d})
			}
			keys := make([]string, 0, len(x))
			for key := range x {
				keys = append(keys, key)
			}
			sort.Slice(keys, func(i, j int) bool {
				if (keys[i] == "derivatives") != (keys[j] == "derivatives") {
					return keys[i] == "derivatives"
				}
				return keys[i] < keys[j]
			})
			for _, key := range keys {
				walk(x[key], size, variant || key == "derivatives", awaiting)
			}
		case []any:
			for _, child := range x {
				walk(child, size, variant, awaiting)
			}
		}
	}
	walk(value, 0, false, false)
	return out
}

type BlobEntry struct {
	RetryAttempts int       `json:"retry_attempts,omitempty"`
	NextAttemptAt time.Time `json:"next_attempt_at,omitempty"`
	ResumedBytes  int64     `json:"resumed_bytes"`
	Corruptions   int       `json:"corruptions"`

	Reason         string    `json:"reason,omitempty"`
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
