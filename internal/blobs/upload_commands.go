package blobs

import (
	"context"
	"os"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func (s *Store) BorrowMessageUploads(ctx context.Context, owner string, files []contracts.MessageFile) (map[string]contracts.BlobReference, func(), error) {
	s.uploadMu.Lock()
	refs := map[string]contracts.BlobReference{}
	for _, file := range files {
		if file.UploadID == "" {
			continue
		}
		path, digest, err := s.resolveMessageUpload(ctx, owner, file.UploadID)
		if err != nil {
			s.uploadMu.Unlock()
			return nil, nil, err
		}
		info, err := os.Stat(path)
		if err != nil {
			s.uploadMu.Unlock()
			return nil, nil, err
		}
		if info.Size() > contracts.MessageFileLimit {
			s.uploadMu.Unlock()
			return nil, nil, contracts.ErrTooLarge
		}
		refs[file.UploadID] = contracts.BlobReference{Digest: digest, Size: info.Size(), State: "present"}
	}
	return refs, s.uploadMu.Unlock, nil
}
