package blobs

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func (s *Store) Upload(ctx context.Context, owner string, r contracts.UploadRequest) (contracts.UploadStatus, error) {
	s.uploadMu.Lock()
	defer s.uploadMu.Unlock()
	var empty contracts.UploadStatus
	if owner == "" || (!strings.HasPrefix(r.ID, "upload_") && !strings.HasPrefix(r.ID, "pin_")) || !contracts.ValidCommandID(r.ID) {
		return empty, contracts.ErrInvalid
	}
	if err := ctx.Err(); err != nil {
		return empty, err
	}
	if r.Size > contracts.MessageFileLimit {
		return empty, contracts.ErrTooLarge
	}
	u, err := s.loadUpload(owner, r.ID)
	if r.Action == "begin" && err == contracts.ErrNotFound {
		if r.Size < -1 || r.Digest != "" && !contracts.ValidDigest(r.Digest) {
			return empty, contracts.ErrInvalid
		}
		u = uploadLease{UploadStatus: contracts.UploadStatus{ID: r.ID, Size: r.Size, State: "uploading", ExpiresAt: time.Now().UTC().Add(contracts.UploadLease)}, Owner: owner, Pin: "upload-pin:" + filepath.Base(s.uploadPath(owner, r.ID)), ExpectedDigest: r.Digest}
		if err = os.MkdirAll(filepath.Dir(s.uploadPath(owner, r.ID)), 0700); err != nil {
			return empty, err
		}
		file, openErr := s.openUploadPart(u, true)
		if openErr != nil {
			return empty, openErr
		}
		if err = file.Close(); err != nil {
			return empty, err
		}
		return u.UploadStatus, s.saveUpload(u)
	}
	if err != nil {
		return empty, err
	}
	switch r.Action {
	case "begin":
		if r.Size != u.Size || r.Digest != u.ExpectedDigest {
			return empty, contracts.ErrConflict
		}
	case "status":
	case "commit":
		if r.Digest != "" && u.ExpectedDigest != "" && r.Digest != u.ExpectedDigest {
			return empty, contracts.ErrCorrupt
		}
		if u.State == "ready" {
			return u.UploadStatus, nil
		}
		if u.Size >= 0 && u.Offset != u.Size {
			return empty, contracts.ErrConflict
		}
		if r.Digest != "" {
			u.ExpectedDigest = r.Digest
		}
		return s.commitUpload(ctx, u)
	case "abandon":
		return u.UploadStatus, s.discardUpload(ctx, u)
	default:
		return empty, contracts.ErrInvalid
	}
	return u.UploadStatus, nil
}
