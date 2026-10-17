package blobs

import (
	"context"
	"crypto/sha256"
	"fmt"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"io"
	"os"
	"path/filepath"
)

func (s *Store) messageUploadPath(owner, id string) string {
	return filepath.Join(s.root, "message-uploads", fmt.Sprintf("%x", sha256.Sum256([]byte(owner+"/"+id))))
}
func (s *Store) StageMessageUpload(ctx context.Context, owner string, input io.Reader) (string, error) {
	digest, pin, err := s.Stage(ctx, input, Expect{})
	if err != nil {
		return "", err
	}
	path := s.messageUploadPath(owner, pin)
	if err = os.MkdirAll(filepath.Dir(path), 0700); err == nil {
		err = os.Link(s.path(digest), path)
	}
	if err != nil {
		s.Release(ctx, pin)
		return "", err
	}
	return pin, nil
}
func (s *Store) ResolveMessageUpload(ctx context.Context, owner, id string) (string, string, error) {
	if err := ctx.Err(); err != nil {
		return "", "", err
	}
	path := s.messageUploadPath(owner, id)
	info, err := os.Lstat(path)
	if err != nil || !info.Mode().IsRegular() {
		return "", "", contracts.ErrNotFound
	}
	file, err := os.Open(path)
	if err != nil {
		return "", "", err
	}
	defer file.Close()
	hash := sha256.New()
	if _, err = io.Copy(hash, file); err != nil {
		return "", "", err
	}
	path, err = filepath.EvalSymlinks(path)
	return path, fmt.Sprintf("%x", hash.Sum(nil)), err
}
func (s *Store) ReleaseMessageUpload(ctx context.Context, owner, id string) {
	os.Remove(s.messageUploadPath(owner, id))
	s.Release(ctx, id)
}
