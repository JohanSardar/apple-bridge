package trust

import "github.com/JohanSardar/apple-bridge/internal/contracts"

type RecordGrant struct {
	record  string
	digests map[string]bool
}

func GrantRecord(r contracts.Record) RecordGrant {
	g := RecordGrant{record: r.ID, digests: map[string]bool{}}
	if !r.Deleted {
		for _, ref := range contracts.BlobReferences(r.Data) {
			g.digests[ref.Digest] = true
		}
	}
	return g
}
func (g RecordGrant) AllowsBlob(d string) bool { return g.record != "" && g.digests[d] }
