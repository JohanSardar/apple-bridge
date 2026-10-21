package operations

import (
	"context"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

func (s *Service) ThreadWindowCounts(ctx context.Context, p trust.Principal, window contracts.ThreadWindow) (contracts.WindowCounts, error) {
	return s.State.ThreadWindowCounts(ctx, p.Visibility(), window)
}
