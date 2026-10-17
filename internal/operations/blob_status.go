package operations

import (
	"context"
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/blobs"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

func (s *Service) LocalReplicaStatus(ctx context.Context, p trust.Principal) (any, error) {
	if s.LocalStatus == nil {
		return nil, nil
	}
	b, err := json.Marshal(s.LocalStatus())
	if err != nil {
		return nil, err
	}
	var status map[string]json.RawMessage
	if err = json.Unmarshal(b, &status); err != nil {
		return nil, err
	}
	var progress blobs.BlobProgress
	if err = json.Unmarshal(status["blobs"], &progress); err != nil {
		return nil, err
	}
	allowed := map[string]bool{}
	after := ""
	for {
		result, err := s.State.Read(ctx, p.Visibility(), contracts.ReadQuery{After: after})
		if err != nil {
			return nil, err
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
	visible := blobs.BlobProgress{Entries: map[string]blobs.Entry{}}
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
	status["blobs"], err = json.Marshal(visible)
	return status, err
}
