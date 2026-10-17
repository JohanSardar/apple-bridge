//go:build !lab

package runtime

import (
	"context"
	"github.com/JohanSardar/apple-bridge/internal/blobs"
	"github.com/JohanSardar/apple-bridge/internal/state"
)

func serveBlobLab(ctx context.Context, home string, s *state.Store, b *blobs.Store) error { return nil }
