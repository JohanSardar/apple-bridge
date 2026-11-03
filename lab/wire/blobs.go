package wire

import (
	"encoding/json"
	"sort"
)

type DigestReference struct {
	Digest string `json:"digest"`
}

func BlobReferences(data json.RawMessage) []DigestReference {
	var decoded any
	if json.Unmarshal(data, &decoded) != nil {
		return nil
	}
	result := []DigestReference{}
	var visit func(any)
	visit = func(value any) {
		switch fields := value.(type) {
		case map[string]any:
			for _, name := range []string{"digest", "raw_body_digest"} {
				if digest, ok := fields[name].(string); ok && validDigest(digest) {
					result = append(result, DigestReference{Digest: digest})
				}
			}
			visit(fields["derivatives"])
			names := make([]string, 0, len(fields))
			for name := range fields {
				if name != "derivatives" {
					names = append(names, name)
				}
			}
			sort.Strings(names)
			for _, name := range names {
				visit(fields[name])
			}
		case []any:
			for _, value := range fields {
				visit(value)
			}
		}
	}
	visit(decoded)
	return result
}

func validDigest(value string) bool {
	if len(value) != 64 {
		return false
	}
	for _, letter := range value {
		if !(letter >= '0' && letter <= '9' || letter >= 'a' && letter <= 'f') {
			return false
		}
	}
	return true
}
