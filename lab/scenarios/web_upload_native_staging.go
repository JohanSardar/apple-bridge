package scenarios

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func uploadNativeStaging(ctx context.Context, w *world.World, c *webClient, e *Evidence, add func(string, bool)) {
	run := e.RunID
	data := []byte("[ab-test " + run + "] private web attachment")
	sum := sha256.Sum256(data)
	status, ready := uploadRequest(c, "POST", "upload", bytes.NewReader(data), map[string]string{"Upload-Digest": fmt.Sprintf("%x", sum)})
	add("web_upload_verified_ready", status == 200 && ready.State == "ready")
	if ready.ID == "" {
		return
	}
	socket, err := c.socket()
	add("web_command_channel", err == nil)
	if err != nil {
		return
	}
	defer socket.CloseNow()
	args, _ := json.Marshal(map[string]any{"handles": []string{"self@example.com"}, "service": "iMessage", "text": "[ab-test " + run + "] attachment", "files": []contracts.MessageFile{{Name: "ab-" + run + ".txt", UploadID: ready.ID}}})
	command := contracts.Command{ID: contracts.NewCommandID(), Op: "messages.send", Args: args}
	record, err := uploadCommandReply(ctx, socket, command)
	if current, readErr := uploadGatewayCommand(ctx, w.Gateway.Home, command.ID); readErr == nil {
		record = current
	}
	e.Metrics["native_command_state"] = record.State
	if record.Error != nil {
		e.Metrics["native_command_error_code"] = record.Error.Code
	}
	add("native_boundary_reached", record.State == contracts.CommandObserved || record.Error != nil && record.Error.Code == "invalid")
	add("web_uploaded_attachment_observed_through_native_boundary", err == nil && record.State == contracts.CommandObserved)
	effects, effectErr := w.Effects()
	add("exactly_one_attachment_command_effect", effectErr == nil && effects[command.ID] == 1)
	refs := record.UploadReferences
	add("command_retains_verified_digest", refs[ready.ID].Digest == fmt.Sprintf("%x", sum))
	add("command_staging_cleaned_after_dispatch", uploadCommandStagingEmpty(w))
	_, statErr := os.Stat(filepath.Join(w.Gateway.Home, "blobs", "sha256", fmt.Sprintf("%x", sum)[:2], fmt.Sprintf("%x", sum)[2:]))
	add("protected_blob_survives_staging_cleanup", statErr == nil)
	uploadNativeCorrupt(ctx, w, c, run, add)
	root := filepath.Join(w.Gateway.Home, "blobs", "command-uploads")
	fresh := filepath.Join(root, "fresh", "message-uploads", "Bridge Test.txt")
	old := filepath.Join(root, "old", "message-uploads", "Bridge Test.txt")
	for _, path := range []string{fresh, old} {
		err = os.MkdirAll(filepath.Dir(path), 0700)
		if err == nil {
			err = os.WriteFile(path, []byte("Bridge Test interrupted handoff"), 0600)
		}
		add("interrupted_command_staging_fixture", err == nil)
		if err != nil {
			return
		}
	}
	oldTime := time.Now().Add(-2 * time.Hour)
	err = os.Chtimes(filepath.Join(root, "old"), oldTime, oldTime)
	add("old_handoff_age_fixture", err == nil)
	if err != nil {
		return
	}
	err = w.RestartGateway()
	add("gateway_restarted_after_interrupted_staging", err == nil)
	_, freshErr := os.Stat(fresh)
	_, oldErr := os.Stat(old)
	add("fresh_handoff_survives_restart", err == nil && freshErr == nil)
	add("startup_removes_only_old_command_staging", err == nil && os.IsNotExist(oldErr))
}

func uploadCommandStagingEmpty(w *world.World) bool {
	return w.Await(5*time.Second, func() bool {
		paths, err := filepath.Glob(filepath.Join(w.Gateway.Home, "blobs", "command-uploads", "*"))
		return err == nil && len(paths) == 0
	}) == nil
}
