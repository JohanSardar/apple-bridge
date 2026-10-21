package blobs

import (
	"context"
	"os"
	"path/filepath"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func (s *Store) AppendUpload(ctx context.Context, owner, id string, offset int64, data []byte) (contracts.UploadStatus, error) {
	s.uploadMu.Lock()
	defer s.uploadMu.Unlock()
	var empty contracts.UploadStatus
	if err := ctx.Err(); err != nil {
		return empty, err
	}
	u, err := s.loadUpload(owner, id)
	if err != nil {
		return empty, err
	}
	if offset != u.Offset || u.State != "uploading" {
		return u.UploadStatus, contracts.ErrConflict
	}
	if len(data) == 0 || len(data) > contracts.BlobChunkSize {
		return u.UploadStatus, contracts.ErrInvalid
	}
	next := offset + int64(len(data))
	if next > contracts.MessageFileLimit {
		return u.UploadStatus, contracts.ErrTooLarge
	}
	if u.Size >= 0 && next > u.Size {
		return u.UploadStatus, contracts.ErrConflict
	}
	file, err := os.OpenFile(s.uploadPath(owner, id)+".part", os.O_CREATE|os.O_RDWR, 0600)
	if err != nil {
		return empty, err
	}
	defer file.Close()
	if err = file.Truncate(offset); err == nil {
		_, err = file.WriteAt(data, offset)
	}
	if err == nil {
		err = file.Sync()
	}
	if err != nil {
		return empty, err
	}
	u.Offset = next
	u.ExpiresAt = time.Now().UTC().Add(contracts.UploadLease)
	return u.UploadStatus, s.saveUpload(u)
}
func (s *Store) commitUpload(ctx context.Context, u uploadLease) (contracts.UploadStatus, error) {
	path := s.uploadPath(u.Owner, u.ID) + ".part"
	file, err := os.OpenFile(path, os.O_CREATE|os.O_RDWR, 0600)
	if err != nil {
		return u.UploadStatus, err
	}
	defer file.Close()
	if err = file.Truncate(u.Offset); err != nil {
		return u.UploadStatus, err
	}
	digest, _, err := s.stage(ctx, file, Expect{Digest: u.ExpectedDigest, Size: u.Offset}, u.Pin)
	if err != nil {
		if err == contracts.ErrCorrupt {
			s.discardUpload(ctx, u)
		}
		return u.UploadStatus, err
	}
	alias := s.messageUploadPath(u.Owner, u.ID)
	if err = os.MkdirAll(filepath.Dir(alias), 0700); err == nil {
		err = os.Link(s.path(digest), alias)
		if os.IsExist(err) {
			err = nil
		}
	}
	if err != nil {
		return u.UploadStatus, err
	}
	u.State = "ready"
	u.Size = u.Offset
	u.Digest = digest
	if err = s.saveUpload(u); err != nil {
		return u.UploadStatus, err
	}
	os.Remove(path)
	return u.UploadStatus, nil
}
