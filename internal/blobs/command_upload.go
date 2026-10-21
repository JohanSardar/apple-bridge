package blobs

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"io"
	"os"
	"path/filepath"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func (s *Store) CommandUpload(ctx context.Context, command, name string, ref contracts.BlobReference) (path string, cleanup func(), err error) {
	if !contracts.ValidCommandID(command) || name == "" || name == "." || name == ".." || filepath.Base(name) != name || !contracts.ValidDigest(ref.Digest) || ref.Size < 0 || ref.Size > contracts.MessageFileLimit {
		return "", nil, contracts.ErrInvalid
	}
	if err = ctx.Err(); err != nil {
		return "", nil, err
	}
	source, err := os.Open(s.path(ref.Digest))
	if err != nil {
		return "", nil, contracts.ErrNotFound
	}
	defer source.Close()
	info, err := source.Stat()
	if err != nil || !info.Mode().IsRegular() || info.Size() != ref.Size {
		return "", nil, contracts.ErrNotFound
	}
	root := filepath.Join(s.root, "command-uploads")
	if err = os.MkdirAll(root, 0700); err != nil {
		return "", nil, err
	}
	directory, err := os.MkdirTemp(root, command+"-")
	if err != nil {
		return "", nil, err
	}
	cleanup = func() { os.RemoveAll(directory) }
	defer func() {
		if err != nil {
			cleanup()
		}
	}()
	parent := filepath.Join(directory, "message-uploads")
	if err = os.Mkdir(parent, 0700); err != nil {
		return "", cleanup, err
	}
	if err = os.Chmod(parent, 0700); err != nil {
		return "", cleanup, err
	}
	path = filepath.Join(parent, name)
	staged, err := os.OpenFile(path, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0600)
	if err != nil {
		return "", cleanup, err
	}
	defer staged.Close()
	if err = staged.Chmod(0600); err != nil {
		return "", cleanup, err
	}
	hash := sha256.New()
	var written int64
	written, err = io.Copy(io.MultiWriter(staged, hash), io.LimitReader(source, ref.Size+1))
	if err != nil {
		return "", cleanup, err
	}
	if written != ref.Size || hex.EncodeToString(hash.Sum(nil)) != ref.Digest {
		return "", cleanup, contracts.ErrCorrupt
	}
	if err = ctx.Err(); err != nil {
		return "", cleanup, err
	}
	if err = staged.Sync(); err != nil {
		return "", cleanup, err
	}
	err = staged.Close()
	return path, cleanup, err
}
