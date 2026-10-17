package blobs

import (
	"context"
	"github.com/JohanSardar/apple-bridge/internal/applepeer"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

type appleSource struct {
	wire   *applepeer.Wire
	handle string
}

func (s appleSource) Read(ctx context.Context, d string, offset, length int64) ([]byte, bool, error) {
	return s.wire.ReadBlob(ctx, s.handle, offset, length)
}
func (s *Store) Capture(ctx context.Context, c contracts.Capture, w *applepeer.Wire) {
	for d, h := range c.BlobHandles {
		s.Transfer(ctx, d, appleSource{w, h})
	}
}
