package scenarios

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"strings"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/operations"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func uploadFeedProof(ctx context.Context, w *world.World, e *Evidence, add func(string, bool)) {
	probe, err := newUploadProbe(ctx, w)
	add("independent_scoped_feed_client", err == nil)
	if err != nil {
		return
	}
	defer probe.close()
	data := bytes.Repeat([]byte("Bridge Test binary upload\n"), 100000)
	hash := sha256.Sum256(data)
	digest := hex.EncodeToString(hash[:])
	_, err = probe.control(ctx, contracts.UploadRequest{Action: "begin", ID: contracts.NewID("upload"), Size: 10})
	add("gateway_requires_forwarded_begin_digest", errors.Is(err, contracts.ErrInvalid))
	_, err = probe.control(ctx, contracts.UploadRequest{Action: "begin", ID: contracts.NewID("upload"), Size: contracts.MessageFileLimit + 1, Digest: digest})
	add("gateway_independently_rejects_oversize", errors.Is(err, contracts.ErrTooLarge))
	id := contracts.NewID("upload")
	status, err := probe.control(ctx, contracts.UploadRequest{Action: "begin", ID: id, Size: int64(len(data)), Digest: digest})
	add("signed_upload_begin_accepted", err == nil && status.State == "uploading")
	if err != nil {
		return
	}
	uploadBeforeReady(ctx, w, probe, id, add)
	first, err := probe.chunk(ctx, id, digest, 0, data[:contracts.BlobChunkSize])
	add("one_MiB_binary_frame_acknowledged", err == nil && first.Offset == contracts.BlobChunkSize)
	if err != nil {
		return
	}
	resumes := probe.client.Status().Resumes
	_, err = probe.client.UploadChunk(ctx, contracts.NewID("replay"), digest, 0, data[:contracts.BlobChunkSize])
	afterReplay := probe.client.Status()
	add("captured_binary_chunk_replay_rejected", err != nil && (afterReplay.State != "connected" || afterReplay.Resumes > resumes))
	err = probe.connect(ctx, w)
	add("same_paired_identity_reconnected", err == nil)
	if err != nil {
		return
	}
	status, err = probe.control(ctx, contracts.UploadRequest{Action: "status", ID: id})
	add("resume_offset_survives_connection_loss", err == nil && status.Offset == first.Offset)
	add("status_does_not_extend_gateway_lease", err == nil && status.ExpiresAt.Equal(first.ExpiresAt))
	if err != nil {
		return
	}
	for status.Offset < int64(len(data)) {
		offset := status.Offset
		end := min(int64(len(data)), offset+contracts.BlobChunkSize)
		status, err = probe.chunk(ctx, id, digest, offset, data[offset:end])
		if err != nil {
			break
		}
	}
	add("remaining_binary_chunks_acknowledged", err == nil && status.Offset == int64(len(data)))
	if err != nil {
		return
	}
	_, err = probe.control(ctx, contracts.UploadRequest{Action: "commit", ID: id})
	add("gateway_requires_forwarded_commit_digest", errors.Is(err, contracts.ErrInvalid))
	_, wrongErr := probe.control(ctx, contracts.UploadRequest{Action: "commit", ID: id, Digest: strings.Repeat("0", 64)})
	pending, pendingErr := probe.control(ctx, contracts.UploadRequest{Action: "status", ID: id})
	add("changed_commit_digest_preserves_uploading_lease", errors.Is(wrongErr, contracts.ErrCorrupt) && pendingErr == nil && pending.State == "uploading" && pending.Offset == int64(len(data)))
	status, err = probe.control(ctx, contracts.UploadRequest{Action: "commit", ID: id, Digest: digest})
	add("gateway_digest_verified_before_ready", err == nil && status.State == "ready" && status.Digest == digest && status.Size == int64(len(data)))
	raw, readErr := os.ReadFile(filepath.Join(w.Gateway.Home, "blobs/sha256", digest[:2], digest[2:]))
	add("resumed_gateway_bytes_exact", readErr == nil && bytes.Equal(raw, data))
	_, err = probe.control(ctx, contracts.UploadRequest{Action: "commit", ID: id, Digest: strings.Repeat("0", 64)})
	add("ready_commit_rejects_changed_digest", errors.Is(err, contracts.ErrCorrupt))
	retried, err := probe.control(ctx, contracts.UploadRequest{Action: "commit", ID: id, Digest: digest})
	add("ready_commit_retry_keeps_verified_lease", err == nil && retried.Digest == digest && retried.State == "ready" && retried.ExpiresAt.Equal(status.ExpiresAt))
	e.Metrics["resumed_feed_upload_bytes"] = len(raw)
	uploadFeedCrossSession(ctx, w, probe, add)
	uploadFeedCorruption(ctx, w, probe, add)
}

func uploadFeedCorruption(ctx context.Context, w *world.World, probe *uploadProbe, add func(string, bool)) {
	data := bytes.Repeat([]byte("x"), contracts.BlobChunkSize)
	hash := sha256.Sum256(data)
	digest := hex.EncodeToString(hash[:])
	data[0] = 'y'
	id := contracts.NewID("upload")
	before, err := uploadCASCount(w.Gateway.Home)
	add("gateway_corruption_inventory", err == nil)
	_, err = probe.control(ctx, contracts.UploadRequest{Action: "begin", ID: id, Size: int64(len(data)), Digest: digest})
	add("declared_digest_begin_accepted", err == nil)
	if err != nil {
		return
	}
	_, err = probe.chunk(ctx, id, digest, 0, data)
	add("untrusted_bytes_received_before_verification", err == nil)
	if err != nil {
		return
	}
	_, err = probe.control(ctx, contracts.UploadRequest{Action: "commit", ID: id, Digest: digest})
	add("gateway_rejects_corrupted_forwarded_bytes", errors.Is(err, contracts.ErrCorrupt))
	after, err := uploadCASCount(w.Gateway.Home)
	add("corrupt_forwarding_never_staged", err == nil && before == after)
	_, err = probe.control(ctx, contracts.UploadRequest{Action: "status", ID: id})
	add("corrupt_upload_lease_discarded", errors.Is(err, contracts.ErrNotFound))
}

func uploadBeforeReady(ctx context.Context, w *world.World, probe *uploadProbe, id string, add func(string, bool)) {
	args, _ := json.Marshal(map[string]any{"handles": []string{"self@example.com"}, "service": "iMessage", "text": "[ab-test upload-not-ready]", "files": []contracts.MessageFile{{Name: "Bridge Test.txt", UploadID: id}}})
	command := contracts.Command{ID: contracts.NewCommandID(), Op: "messages.send", Args: args}
	remote := &operations.RemoteGateway{Authority: probe.authority, Exchange: probe.client.Exchange, Connected: func() bool { return probe.client.Status().State == "connected" }, Wait: func(context.Context, contracts.Cursor) error { return nil }}
	record, err := remote.Submit(ctx, probe.principal, command)
	add("incomplete_upload_command_not_admitted", errors.Is(err, contracts.ErrConflict) || record.State == contracts.CommandRejected && record.Error != nil && errors.Is(record.Error, contracts.ErrConflict))
	effects, err := w.Effects()
	add("incomplete_upload_has_no_effect", err == nil && effects[command.ID] == 0)
}
