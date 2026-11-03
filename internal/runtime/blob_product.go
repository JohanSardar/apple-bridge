//go:build !lab

package runtime

import (
	"context"
	"github.com/JohanSardar/apple-bridge/internal/blobs"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/state"
)

func serveBlobLab(ctx context.Context, home string, s *state.Store, b *blobs.Store) error { return nil }

func phoneApprovalObserver(_ string, decide func(context.Context, string, string, bool) (contracts.CommandRecord, error)) func(context.Context, string, string, bool) (contracts.CommandRecord, error) {
	return decide
}
