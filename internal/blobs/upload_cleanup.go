package blobs

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func (s *Store) collectUploads(ctx context.Context) {
	s.uploadMu.Lock()
	defer s.uploadMu.Unlock()
	if time.Since(s.uploadSweep) < time.Minute {
		return
	}
	s.uploadSweep = time.Now()
	entries, _ := os.ReadDir(filepath.Join(s.root, "upload-leases"))
	for _, entry := range entries {
		if !strings.HasSuffix(entry.Name(), ".json") {
			continue
		}
		raw, err := os.ReadFile(filepath.Join(s.root, "upload-leases", entry.Name()))
		var u uploadLease
		if err == nil && json.Unmarshal(raw, &u) == nil && !time.Now().Before(u.ExpiresAt) {
			s.discardUpload(ctx, u)
		}
	}
}
func (s *Store) releaseUploadLease(ctx context.Context, owner, id string) bool {
	s.uploadMu.Lock()
	defer s.uploadMu.Unlock()
	raw, err := os.ReadFile(s.uploadPath(owner, id) + ".json")
	if os.IsNotExist(err) {
		return false
	}
	var u uploadLease
	if err == nil && json.Unmarshal(raw, &u) == nil && u.Owner == owner && u.ID == id && u.Pin != "" {
		s.discardUpload(ctx, u)
	}
	return true
}
func (s *Store) checkUploadLease(owner, id string) error {
	if !strings.HasPrefix(id, "upload_") && !strings.HasPrefix(id, "pin_") {
		return nil
	}
	u, err := s.loadUpload(owner, id)
	if err != nil {
		if strings.HasPrefix(id, "pin_") && err == contracts.ErrNotFound {
			if _, legacyErr := os.Lstat(s.messageUploadPath(owner, id)); legacyErr == nil {
				return nil
			}
		}
		return err
	}
	if u.State != "ready" {
		return contracts.ErrConflict
	}
	return nil
}
