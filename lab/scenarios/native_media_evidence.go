package scenarios

import (
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"os"
	"path/filepath"
	"strings"
	"time"
)

func nativeMediaEvidence(w *world.World, e *Evidence, effects []any) {
	rows := readNativeTrace(filepath.Join(w.Gateway.Home, "helper", "media-work-trace.jsonl"))
	first := effects[0].(map[string]any)
	last := effects[len(effects)-1].(map[string]any)
	start, _ := first["effect_uptime_ns"].(json.Number).Int64()
	end, _ := last["effect_uptime_ns"].(json.Number).Int64()
	counts, peaks := map[string]int{}, map[string]int{}
	active, occupied := map[string]int64{}, map[string]int64{}
	waiting := []float64{}
	pauseStates := []map[string]any{}
	interval := func(name string, from, through int64) {
		if through > start && from < end {
			occupied[name] += max(0, min(end, through)-max(start, from))
		}
	}
	for _, row := range rows {
		counts[row.Stage]++
		if row.Stage == "media_paused" || row.Stage == "media_resumed" {
			pauseStates = append(pauseStates, map[string]any{"lab_paused": row.LabPaused, "system_paused": row.SystemPaused, "thermal": row.Thermal})
		}
		peaks["media"] = max(peaks["media"], row.PendingMedia)
		peaks["hash"] = max(peaks["hash"], row.PendingBlobs)
		if row.Stage == "media_started" && row.QueuedNS > 0 {
			waiting = append(waiting, float64(row.UptimeNS-row.QueuedNS)/float64(time.Millisecond))
		}
		if name, ok := strings.CutSuffix(row.Stage, "_started"); ok && strings.HasSuffix(name, "_compute") {
			active[name] = row.UptimeNS
		}
		if name, ok := strings.CutSuffix(row.Stage, "_finished"); ok && active[name] > 0 {
			interval(name, active[name], row.UptimeNS)
			delete(active, name)
		}
	}
	for name, at := range active {
		interval(name, at, end)
	}
	if e.Metrics == nil {
		e.Metrics = map[string]any{}
	}
	e.Metrics["media_backfill_paused"] = os.Getenv("APPLE_BRIDGE_LAB_PAUSE_MEDIA") == "1"
	e.Metrics["media_work_stage_counts"] = counts
	e.Metrics["media_pause_states"] = pauseStates
	e.Metrics["media_pending_peak"] = peaks
	e.Metrics["media_compute_during_burst_ms"] = float64(occupied["media_compute"]) / float64(time.Millisecond)
	e.Metrics["attachment_prepare_during_burst_ms"] = float64(occupied["attachment_prepare_compute"]) / float64(time.Millisecond)
	e.Metrics["hash_compute_during_burst_ms"] = float64(occupied["hash_compute"]) / float64(time.Millisecond)
	blobReads := map[string][]float64{}
	for _, row := range readNativeTrace(filepath.Join(w.Gateway.Home, "helper", "capture-trace.jsonl")) {
		if row.Stage == "helper_blob_read" && row.FinishedNS > start && row.StartedNS < end {
			blobReads[row.Source] = append(blobReads[row.Source], float64(row.FinishedNS-row.StartedNS)/float64(time.Millisecond))
		}
	}
	for source, values := range blobReads {
		e.Latency["helper_blob_read:"+source] = stats(values, 0)
	}
	e.Latency["media_backfill_queue_wait"] = stats(waiting, 0)
}
