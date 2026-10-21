package blobs

import (
	"context"
	"crypto/sha256"
	"fmt"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"io"
	"os"
	"path/filepath"
	"time"
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
		s.Release(context.WithoutCancel(ctx), pin)
		return "", err
	}
	s.uploadMu.Lock()
	info, statErr := os.Stat(path)
	if statErr == nil {
		err = s.saveUpload(uploadLease{UploadStatus: contracts.UploadStatus{ID: pin, Offset: info.Size(), Size: info.Size(), State: "ready", ExpiresAt: time.Now().UTC().Add(contracts.UploadLease), Digest: digest}, Owner: owner, Pin: pin, ExpectedDigest: digest})
	} else {
		err = statErr
	}
	s.uploadMu.Unlock()
	if err != nil {
		os.Remove(path)
		s.Release(context.WithoutCancel(ctx), pin)
		return "", err
	}
	return pin, nil
}
func (s *Store) ResolveMessageUpload(ctx context.Context, owner, id string) (string, string, error) {
	s.uploadMu.Lock()
	defer s.uploadMu.Unlock()
	return s.resolveMessageUpload(ctx, owner, id)
}
func (s *Store) resolveMessageUpload(ctx context.Context, owner, id string) (string, string, error) {
	if err := s.checkUploadLease(owner, id); err != nil {
		return "", "", err
	}
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
	if s.releaseUploadLease(ctx, owner, id) {
		return
	}
	path := s.messageUploadPath(owner, id)
	if info, err := os.Lstat(path); err != nil || !info.Mode().IsRegular() {
		return
	}
	os.Remove(path)
	s.Release(context.WithoutCancel(ctx), id)
}
