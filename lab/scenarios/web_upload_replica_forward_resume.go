package scenarios

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"os"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func uploadReplicaForwardResume(w *world.World, c *webClient, data []byte, e *Evidence, add func(string, bool)) (int, uploadResult) {
	status, started := uploadRequest(c, "POST", "upload", nil, map[string]string{"Upload-Length": fmt.Sprint(len(data))})
	add("forward_http_upload_id_retained_before_bytes", status == 200 && started.ID != "")
	if status != 200 {
		return status, started
	}
	w.DelayBothDirections(50 * time.Millisecond)
	defer w.DelayBothDirections(0)
	bounded, cancel := context.WithCancel(c.ctx)
	defer cancel()
	interrupted := *c
	interrupted.ctx = bounded
	done := make(chan struct{})
	go func() {
		defer close(done)
		uploadRequest(&interrupted, "POST", "upload", bytes.NewReader(data), map[string]string{"Upload-ID": started.ID, "Upload-Offset": "0"})
	}()
	var prefix contracts.UploadStatus
	err := w.Await(15*time.Second, func() bool {
		path := uploadLeaseFixturePath(w.Gateway.Home, started.ID)
		raw, _ := os.ReadFile(path)
		return json.Unmarshal(raw, &prefix) == nil && prefix.Offset >= contracts.BlobChunkSize && prefix.Offset < prefix.Size && prefix.State == "uploading"
	})
	cancel()
	<-done
	add("final_append_interrupted_during_gateway_forward", err == nil)
	if err != nil {
		return 0, started
	}
	var recovered uploadResult
	err = w.Await(5*time.Second, func() bool {
		status, recovered = uploadRequest(c, "GET", "upload/"+started.ID, nil, nil)
		return status == 200
	})
	add("http_status_reports_uploading_with_all_browser_bytes", err == nil && recovered.State == "uploading" && recovered.Offset == int64(len(data)) && recovered.Size == int64(len(data)))
	e.Metrics["interrupted_gateway_forward_offset_bytes"] = prefix.Offset
	w.DelayBothDirections(0)
	status, recovered = uploadRequest(c, "POST", "upload", bytes.NewReader(nil), map[string]string{"Upload-ID": started.ID, "Upload-Offset": fmt.Sprint(len(data))})
	add("zero_byte_final_append_resumes_until_gateway_verified", status == 200 && recovered.ID == started.ID && recovered.State == "ready")
	return status, recovered
}
