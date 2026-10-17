package operations

import (
	"context"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/state"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

type Service struct {
	State       *state.Store
	LocalStatus func() any
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
	case "sources.status":
		q = contracts.ReadQuery{Collection: "system.source"}
	default:
		return contracts.ReadResult{}, contracts.ErrUnsupported
	}
	return s.State.Read(ctx, p.Visibility(), q)
}
func (s *Service) Head() contracts.Cursor { return s.State.Head() }
func (s *Service) Observe(ctx context.Context, p trust.Principal, after contracts.Cursor) (*state.ChangeIterator, error) {
	return s.State.Changes(ctx, p.Visibility(), after)
}
