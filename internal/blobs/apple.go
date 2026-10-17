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

type appleJob struct {
	ctx    context.Context
	source appleSource
}

func (s *Store) Capture(ctx context.Context, c contracts.Capture, w *applepeer.Wire) {
	if len(c.BlobHandles) == 0 {
		return
	}
	s.appleMu.Lock()
	defer s.appleMu.Unlock()
	if s.applePending == nil {
		s.applePending = map[string]appleJob{}
	}
	for d, h := range c.BlobHandles {
		if contracts.ValidDigest(d) {
			s.applePending[d] = appleJob{ctx, appleSource{w, h}}
		}
	}
	if !s.appleRunning {
		s.appleRunning = true
		go s.captureBlobs()
	}
}
func (s *Store) captureBlobs() {
	for {
		s.appleMu.Lock()
		var digest string
		var job appleJob
		for d, j := range s.applePending {
			digest, job = d, j
			delete(s.applePending, d)
			break
		}
		if digest == "" {
			s.appleRunning = false
			s.appleMu.Unlock()
			return
		}
		s.appleMu.Unlock()
		s.Transfer(job.ctx, digest, job.source)
	}
}
