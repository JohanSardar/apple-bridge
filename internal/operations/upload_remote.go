package operations

import (
	"context"
	"io"
	"os"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

func (g *RemoteGateway) lockUpload(ctx context.Context) (func(), error) {
	g.uploadMu.Lock()
	if g.uploadSlot == nil {
		g.uploadSlot = make(chan struct{}, 1)
	}
	slot := g.uploadSlot
	g.uploadMu.Unlock()
	select {
	case slot <- struct{}{}:
		return func() { <-slot }, nil
	case <-ctx.Done():
		return nil, ctx.Err()
	}
}
func (g *RemoteGateway) ForwardUpload(ctx context.Context, p trust.Principal, local contracts.UploadStatus) (contracts.UploadStatus, error) {
	unlock, err := g.lockUpload(ctx)
	if err != nil {
		return local, err
	}
	defer unlock()
	if g.Uploads == nil || g.UploadBytes == nil {
		return local, contracts.ErrUnavailable
	}
	path, digest, err := g.Uploads.ResolveMessageUpload(ctx, p.ID, local.ID)
	if err != nil {
		return local, err
	}
	file, err := os.Open(path)
	if err != nil {
		return local, err
	}
	defer file.Close()
	info, err := file.Stat()
	if err != nil {
		return local, err
	}
	if local.State != "ready" || !contracts.ValidDigest(local.Digest) || local.Digest != digest || local.Size != info.Size() || local.Offset != info.Size() {
		return local, contracts.ErrCorrupt
	}
	if info.Size() > contracts.MessageFileLimit {
		return local, contracts.ErrTooLarge
	}
	remote, err := g.UploadControl(ctx, p, contracts.UploadRequest{Action: "begin", ID: local.ID, Size: info.Size(), Digest: digest})
	if err != nil {
		return local, err
	}
	if remote.State == "ready" {
		return uploadReady(local, remote, digest)
	}
	buffer := make([]byte, contracts.BlobChunkSize)
	for remote.Offset < info.Size() {
		size := min(int64(len(buffer)), info.Size()-remote.Offset)
		n, err := file.ReadAt(buffer[:size], remote.Offset)
		if err != nil && err != io.EOF {
			return local, err
		}
		if int64(n) != size {
			return local, contracts.ErrCorrupt
		}
		chunk := contracts.UploadRequest{Action: "prepare", ID: local.ID, Offset: remote.Offset, Digest: digest, ChunkID: contracts.NewID("upload-chunk")}
		if _, err = g.UploadControl(ctx, p, chunk); err != nil {
			return local, err
		}
		response, err := g.UploadBytes(ctx, chunk.ChunkID, digest, remote.Offset, buffer[:n])
		if err != nil {
			return local, err
		}
		next, err := decodeUploadResponse(response, local.ID)
		if err != nil {
			return local, err
		}
		if next.Offset != remote.Offset+int64(n) {
			return local, contracts.ErrCorrupt
		}
		remote = next
		if err = uploadForwardCheckpoint(ctx, local.ID, remote.Offset, info.Size()); err != nil {
			return local, err
		}
	}
	remote, err = g.UploadControl(ctx, p, contracts.UploadRequest{Action: "commit", ID: local.ID, Digest: digest})
	if err != nil {
		return local, err
	}
	return uploadReady(local, remote, digest)
}
func uploadReady(local, remote contracts.UploadStatus, digest string) (contracts.UploadStatus, error) {
	if remote.State != "ready" || remote.Digest != digest || remote.Size != local.Size || remote.Offset != local.Size {
		return local, contracts.ErrCorrupt
	}
	if !time.Now().Before(local.ExpiresAt) || !time.Now().Before(remote.ExpiresAt) {
		return local, contracts.ErrExpired
	}
	if local.ExpiresAt.Before(remote.ExpiresAt) {
		remote.ExpiresAt = local.ExpiresAt
	}
	return remote, nil
}
