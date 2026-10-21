package scenarios

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func uploadCheckpointRequest(ctx context.Context, origin, secret, action string, input any) (int, map[string]any) {
	raw, _ := json.Marshal(input)
	request, _ := http.NewRequestWithContext(ctx, "POST", origin+action, bytes.NewReader(raw))
	request.Header.Set("X-Bridge-Lab", secret)
	response, err := http.DefaultClient.Do(request)
	if err != nil {
		return 0, nil
	}
	defer response.Body.Close()
	var result map[string]any
	json.NewDecoder(io.LimitReader(response.Body, 4096)).Decode(&result)
	return response.StatusCode, result
}
func uploadReplicaCheckpoint(w *world.World, c *webClient, data []byte, add func(string, bool)) (int, uploadResult) {
	control, secret := webBrowserControl(c.ctx, w, c.origin)
	defer control.Close()
	status, upload := uploadRequest(c, "POST", "upload", nil, map[string]string{"Upload-Length": fmt.Sprint(len(data))})
	add("checkpoint_upload_initialized", status == 200 && upload.ID != "")
	if status != 200 {
		return status, upload
	}
	input := map[string]string{"upload_id": upload.ID}
	path := "/replica-upload-checkpoint/"
	status, _ = uploadCheckpointRequest(c.ctx, control.URL, "", path+"arm", input)
	add("checkpoint_requires_control_secret", status == 403)
	status, result := uploadCheckpointRequest(c.ctx, control.URL, secret, path+"arm", input)
	add("checkpoint_armed_by_upload_id", status == 200 && result["state"] == "armed")
	if status != 200 {
		return status, upload
	}
	bounded, cancel := context.WithCancel(c.ctx)
	defer cancel()
	interrupted := *c
	interrupted.ctx = bounded
	done := make(chan struct{})
	go func() {
		defer close(done)
		uploadRequest(&interrupted, "POST", "upload", bytes.NewReader(data), map[string]string{"Upload-ID": upload.ID, "Upload-Offset": "0"})
	}()
	wait, stop := context.WithTimeout(c.ctx, 15*time.Second)
	defer stop()
	status, result = uploadCheckpointRequest(wait, control.URL, secret, path+"wait", input)
	offset, _ := result["offset"].(float64)
	add("checkpoint_holds_after_acknowledged_prefix", status == 200 && result["state"] == "paused" && offset == contracts.BlobChunkSize && result["size"] == float64(len(data)))
	status, _ = uploadCheckpointRequest(c.ctx, control.URL, secret, path+"release", input)
	add("checkpoint_cannot_release_uncanceled_request", status == 409)
	cancel()
	<-done
	err := w.Await(5*time.Second, func() bool {
		status, result = uploadCheckpointRequest(c.ctx, control.URL, secret, path+"release", input)
		return status == 200 && result["state"] == "released" && result["canceled"] == true
	})
	add("checkpoint_release_proves_backend_context_canceled", err == nil)
	status, upload = uploadRequest(c, "GET", "upload/"+upload.ID, nil, nil)
	add("checkpoint_http_status_uploading_with_all_browser_bytes", status == 200 && upload.State == "uploading" && upload.Offset == int64(len(data)))
	status, upload = uploadRequest(c, "POST", "upload", bytes.NewReader(nil), map[string]string{"Upload-ID": upload.ID, "Upload-Offset": fmt.Sprint(len(data))})
	add("checkpoint_zero_append_resumes_gateway_verification", status == 200 && upload.State == "ready")
	return status, upload
}
func uploadCheckpointEffect(ctx context.Context, w *world.World, id string, add func(string, bool)) {
	control, secret := webBrowserControl(ctx, w, w.Gateway.URL)
	defer control.Close()
	input := map[string]string{"command_id": id}
	status, _ := uploadCheckpointRequest(ctx, control.URL, "", "/command-effect", input)
	add("effect_oracle_requires_control_secret", status == 403)
	status, result := uploadCheckpointRequest(ctx, control.URL, secret, "/command-effect", input)
	add("effect_oracle_returns_only_selected_count", status == 200 && len(result) == 1 && result["count"] == float64(1))
	status, result = uploadCheckpointRequest(ctx, control.URL, secret, "/command-effect", map[string]string{"command_id": contracts.NewCommandID()})
	add("effect_oracle_unknown_command_has_zero_effects", status == 200 && len(result) == 1 && result["count"] == float64(0))
}
