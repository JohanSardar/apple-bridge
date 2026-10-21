package scenarios

import (
	"context"
	"crypto/sha256"
	"encoding/hex"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func uploadFeedCrossSession(ctx context.Context, w *world.World, probe *uploadProbe, add func(string, bool)) {
	data := []byte("Bridge Test session-bound upload")
	hash := sha256.Sum256(data)
	digest := hex.EncodeToString(hash[:])
	id := contracts.NewID("upload")
	_, err := probe.control(ctx, contracts.UploadRequest{Action: "begin", ID: id, Size: int64(len(data)), Digest: digest})
	if err == nil {
		_, err = probe.control(ctx, contracts.UploadRequest{Action: "prepare", ID: id, Digest: digest, ChunkID: contracts.NewID("upload-chunk")})
	}
	add("old_session_chunk_prepared", err == nil)
	if err != nil {
		return
	}
	err = probe.connect(ctx, w)
	add("prepared_session_replaced", err == nil)
	if err != nil {
		return
	}
	resumes := probe.client.Status().Resumes
	_, err = probe.client.UploadChunk(ctx, contracts.NewID("upload-chunk"), digest, 0, data)
	after := probe.client.Status()
	add("old_session_prepare_cannot_authorize_new_session", err != nil && (after.State != "connected" || after.Resumes > resumes))
	err = probe.connect(ctx, w)
	if err != nil {
		add("cross_session_offset_unchanged", false)
		return
	}
	status, err := probe.control(ctx, contracts.UploadRequest{Action: "status", ID: id})
	add("cross_session_offset_unchanged", err == nil && status.Offset == 0 && status.State == "uploading")
	probe.control(ctx, contracts.UploadRequest{Action: "abandon", ID: id})
}
