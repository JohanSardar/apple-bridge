package blobs

import (
	"context"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func (s *Store) CheckUploadChunk(ctx context.Context, owner string, r contracts.UploadRequest) (contracts.UploadStatus, error) {
	s.uploadMu.Lock()
	defer s.uploadMu.Unlock()
	if err := ctx.Err(); err != nil {
		return contracts.UploadStatus{}, err
	}
	u, err := s.loadUpload(owner, r.ID)
	if err != nil {
		return contracts.UploadStatus{}, err
	}
	if u.State != "uploading" || u.Offset != r.Offset {
		return u.UploadStatus, contracts.ErrConflict
	}
	if !contracts.ValidDigest(r.Digest) || r.Digest != u.ExpectedDigest {
		return u.UploadStatus, contracts.ErrInvalid
	}
	return u.UploadStatus, nil
}
