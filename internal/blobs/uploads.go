package blobs

import (
	"context"
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

type uploadLease struct {
	contracts.UploadStatus
	Pin            string `json:"pin"`
	Owner          string `json:"owner"`
	ExpectedDigest string `json:"expected_digest,omitempty"`
}

func (s *Store) uploadPath(owner, id string) string {
	return filepath.Join(s.root, "upload-leases", fmt.Sprintf("%x", sha256.Sum256([]byte(owner+"/"+id))))
}
func (s *Store) saveUpload(u uploadLease) error {
	path := s.uploadPath(u.Owner, u.ID) + ".json"
	if err := os.MkdirAll(filepath.Dir(path), 0700); err != nil {
		return err
	}
	raw, err := json.Marshal(u)
	if err != nil {
		return err
	}
	if err = os.WriteFile(path+".tmp", raw, 0600); err != nil {
		return err
	}
	if err = os.Rename(path+".tmp", path); err != nil {
		return err
	}
	return syncDir(filepath.Dir(path))
}
func (s *Store) loadUpload(owner, id string) (uploadLease, error) {
	var u uploadLease
	raw, err := os.ReadFile(s.uploadPath(owner, id) + ".json")
	if os.IsNotExist(err) {
		return u, contracts.ErrNotFound
	}
	if err != nil {
		return u, err
	}
	if json.Unmarshal(raw, &u) != nil || u.Owner != owner || u.ID != id || u.Pin == "" {
		return u, contracts.ErrCorrupt
	}
	if !time.Now().Before(u.ExpiresAt) {
		s.discardUpload(context.Background(), u)
		return u, contracts.ErrNotFound
	}
	return u, nil
}
func (s *Store) discardUpload(ctx context.Context, u uploadLease) error {
	if err := s.Release(context.WithoutCancel(ctx), u.Pin); err != nil {
		return err
	}
	path := s.uploadPath(u.Owner, u.ID)
	os.Remove(path + ".json")
	os.Remove(path + ".json.tmp")
	os.Remove(path + ".part")
	os.Remove(s.messageUploadPath(u.Owner, u.ID))
	return nil
}
