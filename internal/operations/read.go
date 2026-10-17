package operations

import (
	"context"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/state"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

type Service struct {
	Target      CommandTarget
	State       *state.Store
	LocalStatus func() contracts.ReplicaStatus
}

func (s *Service) Read(ctx context.Context, p trust.Principal, name string, q contracts.ReadQuery) (contracts.ReadResult, error) {
	if err := contracts.ValidateQuery(q); err != nil {
		return contracts.ReadResult{}, err
	}
	switch name {
	case "records.get":
		if q.Collection == "" || q.ID == "" {
			return contracts.ReadResult{}, contracts.ErrInvalid
		}
	case "records.query":
	case "sources.status", "replica.status":
		if name == "replica.status" && !p.Visibility()[contracts.System] {
			return contracts.ReadResult{}, contracts.ErrUnauthorized
		}
		q = contracts.ReadQuery{Collection: "system.source"}
	default:
		return contracts.ReadResult{}, contracts.ErrUnsupported
	}
	result, err := s.State.Read(ctx, p.Visibility(), q)
	if err == nil && (name == "sources.status" || name == "replica.status") && s.LocalStatus != nil {
		var status contracts.ReplicaStatus
		status, err = s.LocalReplicaStatus(ctx, p)
		result.Replica = &status
	}
	return result, err
}
func (s *Service) Head() contracts.Cursor { return s.State.Head() }
func (s *Service) Observe(ctx context.Context, p trust.Principal, after contracts.Cursor) (*state.ChangeIterator, error) {
	return s.State.Changes(ctx, p.Visibility(), after)
}

func (s *Service) BlobGrant(ctx context.Context, p trust.Principal, id string) (trust.RecordGrant, error) {
	return s.State.RecordGrant(ctx, p.Visibility(), id)
}
