package scenarios

import (
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	bridgeweb "github.com/JohanSardar/apple-bridge/internal/web"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func uploadFeedLatency(ctx context.Context, w *world.World, e *Evidence, add func(string, bool)) {
	err := w.AddReplicaScope("admin")
	add("live_upload_replica_connected", err == nil)
	if err != nil {
		return
	}
	endpoint := w.Replicas[len(w.Replicas)-1]
	authority, err := trust.Open(endpoint.Home)
	if err != nil {
		add("live_upload_session", false)
		return
	}
	token, _, err := authority.CreateDeviceSession("trusted")
	add("live_upload_session", err == nil)
	if err != nil {
		return
	}
	c := &webClient{ctx: ctx, origin: strings.Replace(endpoint.URL, "127.0.0.1", "localhost", 1), cookie: &http.Cookie{Name: bridgeweb.LocalSessionCookie, Value: token}}
	gateway, err := watch(ctx, w.Gateway, false, func(contracts.Record) bool { return true })
	add("upload_gateway_live_probe", err == nil)
	if err != nil {
		return
	}
	defer gateway.close()
	replica, err := watch(ctx, endpoint, false, func(contracts.Record) bool { return true })
	add("upload_replica_live_probe", err == nil)
	if err != nil {
		return
	}
	defer replica.close()
	idle, err := uploadLiveSamples(w, gateway, replica, "idle", nil)
	add("idle_live_samples_complete", err == nil && len(idle) == 20)
	if err != nil {
		return
	}
	e.Latency["idle_gateway_to_replica_live"] = stats(idle, 0)
	type result struct {
		status int
		upload uploadResult
	}
	done := make(chan result, 1)
	start := time.Now()
	go func() {
		status, upload := uploadRequest(c, "POST", "upload", io.LimitReader(webZeroReader{}, contracts.MessageFileLimit), nil)
		done <- result{status, upload}
	}()
	active := func() bool { return uploadFeedActive(w.Gateway.Home) }
	err = w.Await(30*time.Second, active)
	add("hundred_MiB_feed_upload_observed_in_progress", err == nil)
	var busy []float64
	if err == nil {
		busy, err = uploadLiveSamples(w, gateway, replica, "busy", active)
	}
	add("upload_live_samples_complete", err == nil && len(busy) == 20)
	select {
	case uploaded := <-done:
		elapsed := time.Since(start).Seconds()
		add("hundred_MiB_upload_verified_ready", uploaded.status == 200 && uploaded.upload.State == "ready" && uploaded.upload.Size == contracts.MessageFileLimit)
		e.Metrics["replica_http_upload_100_MiB_seconds"] = elapsed
		e.Metrics["replica_http_upload_100_MiB_per_second"] = 100 / elapsed
		uploadRequest(c, "DELETE", "upload/"+uploaded.upload.ID, nil, nil)
	case <-ctx.Done():
		add("hundred_MiB_upload_verified_ready", false)
		return
	}
	uploadConcurrentProof(c, e, add)
	if len(busy) == 20 {
		e.Latency["upload_gateway_to_replica_live"] = stats(busy, 0)
		delta := stats(busy, 0).P95 - stats(idle, 0).P95
		e.Metrics["upload_live_p95_regression_ms"] = delta
		e.Metrics["upload_live_reader_worker_trigger_ms"] = 50
	}
}
func uploadFeedActive(home string) bool {
	paths, _ := filepath.Glob(filepath.Join(home, "blobs/upload-leases/*.json"))
	for _, path := range paths {
		raw, _ := os.ReadFile(path)
		var status contracts.UploadStatus
		if json.Unmarshal(raw, &status) == nil && status.Size == contracts.MessageFileLimit && status.State == "uploading" && status.Offset > 0 && status.Offset < status.Size {
			return true
		}
	}
	return false
}
func uploadLiveSamples(w *world.World, gateway, replica *watcher, phase string, active func() bool) ([]float64, error) {
	values := []float64{}
	for i := 0; i < 20; i++ {
		if active != nil && !active() {
			return values, fmt.Errorf("upload ended before live sample")
		}
		key := contracts.NewID("upload-live-" + phase)
		if err := w.ControlAction("/external-change", map[string]any{"key": key}); err != nil {
			return values, err
		}
		var from, to Seen
		err := w.Await(5*time.Second, func() bool {
			var a, b bool
			from, a = gateway.observed(key)
			to, b = replica.observed(key)
			return a && b
		})
		if err != nil {
			return values, err
		}
		values = append(values, max(0, float64(to.UptimeNS-from.UptimeNS)/1e6))
	}
	return values, nil
}
