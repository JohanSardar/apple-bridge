package state

import (
	"context"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

func (s *Store) FeedChanges(ctx context.Context, visibility trust.Visibility, after contracts.Cursor) (*ChangeIterator, error) {
	iterator, err := s.Changes(ctx, visibility, after)
	if err == nil {
		iterator.parts = true
	}
	return iterator, err
}
