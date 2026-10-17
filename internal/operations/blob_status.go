package operations

import (
	"context"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

func (s *Service) LocalReplicaStatus(ctx context.Context, p trust.Principal) (contracts.ReplicaStatus, error) {
	if s.LocalStatus == nil {
		return contracts.ReplicaStatus{}, nil
	}
	status := s.LocalStatus()
	progress := status.Blobs
	allowed := map[string]bool{}
	after := ""
	for {
		result, err := s.State.Read(ctx, p.Visibility(), contracts.ReadQuery{After: after})
		if err != nil {
			return contracts.ReplicaStatus{}, err
		}
		for _, r := range result.Records {
			for _, ref := range contracts.BlobReferences(r.Data) {
				allowed[ref.Digest] = true
			}
		}
		if result.Next == "" {
			break
		}
		after = result.Next
	}
	visible := contracts.BlobProgress{Entries: map[string]contracts.BlobEntry{}}
	for d, e := range progress.Entries {
		if allowed[d] {
			visible.Entries[d] = e
			if e.State == "present" {
				visible.Bytes += e.Size
			} else {
				visible.Bytes += e.Received
				visible.Backlog++
			}
			visible.ResumedBytes += e.ResumedBytes
			visible.Corruptions += e.Corruptions
		}
	}
	status.Blobs = visible
	return status, nil
}
