package scenarios

import (
	"bytes"
	"context"
	"os"
	"path/filepath"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/operations"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func uploadFeedForwardResume(ctx context.Context, w *world.World, p *uploadProbe, e *Evidence, add func(string, bool)) {
	data := bytes.Repeat([]byte("Bridge Test forward resume\n"), 100000)
	id, err := p.blobs.StageMessageUpload(ctx, p.principal.ID, bytes.NewReader(data))
	add("forward_resume_local_bytes_staged", err == nil)
	if err != nil {
		return
	}
	local, err := p.blobs.Upload(ctx, p.principal.ID, contracts.UploadRequest{Action: "status", ID: id})
	if err != nil {
		add("forward_resume_local_ready", false)
		return
	}
	bounded, cancel := context.WithCancel(ctx)
	defer cancel()
	target := &operations.RemoteGateway{Authority: p.authority, Uploads: p.blobs, UploadExchange: p.client.UploadControl, Connected: func() bool { return p.client.Status().State == "connected" }}
	target.UploadBytes = func(ctx context.Context, id, digest string, offset int64, data []byte) (contracts.CommandResponse, error) {
		response, err := p.client.UploadChunk(ctx, id, digest, offset, data)
		if err == nil && response.Error == nil {
			cancel()
		}
		return response, err
	}
	_, err = target.ForwardUpload(bounded, p.principal, local)
	add("forward_context_cancelled_after_acknowledged_chunk", err != nil && bounded.Err() != nil)
	request := contracts.UploadRequest{Action: "begin", ID: id, Size: local.Size, Digest: local.Digest}
	resumed, err := p.control(ctx, request)
	add("forward_begin_idempotent_at_persisted_gateway_offset", err == nil && resumed.State == "uploading" && resumed.Offset == contracts.BlobChunkSize)
	if err != nil {
		return
	}
	offsets := []int64{}
	target.UploadBytes = func(ctx context.Context, id, digest string, offset int64, data []byte) (contracts.CommandResponse, error) {
		offsets = append(offsets, offset)
		return p.client.UploadChunk(ctx, id, digest, offset, data)
	}
	ready, err := target.ForwardUpload(ctx, p.principal, local)
	add("forward_retry_ready_without_resending_acknowledged_prefix", err == nil && ready.State == "ready" && len(offsets) > 0 && offsets[0] == resumed.Offset)
	raw, readErr := os.ReadFile(filepath.Join(w.Gateway.Home, "blobs/sha256", local.Digest[:2], local.Digest[2:]))
	add("forward_retry_gateway_bytes_exact", readErr == nil && bytes.Equal(raw, data))
	e.Metrics["forward_resume_acknowledged_offset_bytes"] = resumed.Offset
	e.Metrics["forward_resume_chunk_offsets"] = offsets
	p.control(ctx, contracts.UploadRequest{Action: "abandon", ID: id})
}
