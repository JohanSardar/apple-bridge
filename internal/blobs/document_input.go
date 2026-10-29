package blobs

import (
	"context"
	"os"
	"path/filepath"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func (s *Store) DocumentInput(ctx context.Context, data []byte) (path string, cleanup func(), err error) {
	if len(data) == 0 || len(data) > 8<<20 {
		return "", nil, contracts.ErrInvalid
	}
	if err = ctx.Err(); err != nil {
		return "", nil, err
	}
	root := filepath.Join(s.root, "document-inputs")
	if err = os.MkdirAll(root, 0700); err != nil {
		return "", nil, err
	}
	directory, err := os.MkdirTemp(root, "input-")
	if err != nil {
		return "", nil, err
	}
	cleanup = func() { os.RemoveAll(directory) }
	defer func() {
		if err != nil {
			cleanup()
		}
	}()
	path = filepath.Join(directory, "document")
	err = os.WriteFile(path, data, 0600)
	if err == nil {
		err = ctx.Err()
	}
	return path, cleanup, err
}
