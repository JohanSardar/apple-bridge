package scenarios

import (
	"bufio"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/lab/world"
)

type nativeTrace struct {
	Entity          string         `json:"entity"`
	ScopeStartedNS  int64          `json:"scope_started_ns"`
	ScopeFinishedNS int64          `json:"scope_finished_ns"`
	StartedWallNS   float64        `json:"started_wall_ns"`
	FinishedWallNS  float64        `json:"finished_wall_ns"`
	Mode            string         `json:"mode"`
	DecodedRows     int            `json:"decoded_rows"`
	LabPaused       bool           `json:"lab_paused"`
	SystemPaused    bool           `json:"system_paused"`
	Thermal         int            `json:"thermal"`
	PendingMedia    int            `json:"pending_media"`
	PendingBlobs    int            `json:"pending_blobs"`
	QueuedNS        int64          `json:"queued_ns"`
	Source          string         `json:"source"`
	DurationMS      float64        `json:"duration_ms"`
	Histogram       map[string]any `json:"histogram"`

	DeliveredNS         int64    `json:"delivered_ns"`
	CommitStartedWallNS float64  `json:"commit_started_wall_ns"`
	Live                bool     `json:"live"`
	Stage               string   `json:"stage"`
	UptimeNS            int64    `json:"uptime_ns"`
	WallNS              float64  `json:"wall_ns"`
	Keys                []string `json:"keys"`
	Checkpoint          string   `json:"checkpoint"`
	NotificationNS      int64    `json:"notification_ns"`
	DebounceNS          int64    `json:"debounce_ns"`
	StartedNS           int64    `json:"started_ns"`
	FinishedNS          int64    `json:"finished_ns"`
}

type NativeTiming struct {
	CaptureMode            string  `json:"capture_mode,omitempty"`
	ScopeStartedNS         int64   `json:"scope_started_ns,omitempty"`
	ScopeFinishedNS        int64   `json:"scope_finished_ns,omitempty"`
	StoreRefreshStartedNS  int64   `json:"store_refresh_started_ns,omitempty"`
	StoreRefreshFinishedNS int64   `json:"store_refresh_finished_ns,omitempty"`
	StoreRefreshMS         float64 `json:"store_refresh_ms,omitempty"`

	GatewayVisibleWallNS  int64   `json:"gateway_visible_wall_ns"`
	ReplicaVisibleWallNS  int64   `json:"replica_visible_wall_ns"`
	CommitStartedWallNS   float64 `json:"commit_started_wall_ns"`
	TriggerNotificationNS int64   `json:"trigger_notification_ns"`
	SaveNS                int64   `json:"save_ns"`
	NotificationNS        int64   `json:"notification_ns"`
	DatabaseHintNS        int64   `json:"database_hint_ns,omitempty"`
	DebounceNS            int64   `json:"debounce_ns"`
	RefetchStartedNS      int64   `json:"refetch_started_ns"`
	RefetchFinishedNS     int64   `json:"refetch_finished_ns"`
	CaptureSentNS         int64   `json:"capture_sent_ns"`
	CommitAckNS           int64   `json:"commit_ack_ns"`
	SaveWallNS            float64 `json:"save_wall_ns,omitempty"`
	CaptureWallNS         float64 `json:"capture_wall_ns"`
	GatewayCommitWallNS   float64 `json:"gateway_commit_wall_ns"`
	GatewayVisibleNS      int64   `json:"gateway_visible_ns"`
	ReplicaVisibleNS      int64   `json:"replica_visible_ns"`
}

func readNativeTrace(path string) []nativeTrace {
	file, err := os.Open(path)
	if err != nil {
		return nil
	}
	defer file.Close()
	out := []nativeTrace{}
	scanner := bufio.NewScanner(file)
	scanner.Buffer(make([]byte, 4096), 4<<20)
	for scanner.Scan() {
		var row nativeTrace
		if json.Unmarshal(scanner.Bytes(), &row) == nil {
			out = append(out, row)
		}
	}
	return out
}

func nativeLatencyEvidence(w *world.World, e *Evidence, effects []any, observers []*watcher) {
	trace := readNativeTrace(filepath.Join(w.Gateway.Home, "helper", "capture-trace.jsonl"))
	commits := readNativeTrace(filepath.Join(w.Gateway.Home, "capture-trace.jsonl"))
	counts := map[string]int{}
	captureModes := map[string]int{}
	fallbackEntities := map[string]int{}
	calendarRefetch := []float64{}
	for _, event := range trace {
		counts[event.Stage]++
		if event.Stage == "reminders_scope_fallback" {
			fallbackEntities[event.Entity]++
		}
		if event.Stage == "reminders_capture_scope" {
			captureModes[event.Mode]++
		}
		if event.Stage == "refetch" && event.Source == "calendar" && event.FinishedNS >= event.StartedNS {
			calendarRefetch = append(calendarRefetch, float64(event.FinishedNS-event.StartedNS)/float64(time.Millisecond))
		}
	}
	if e.Metrics == nil {
		e.Metrics = map[string]any{}
	}
	e.Metrics["capture_trace_stage_counts"] = counts
	e.Metrics["reminders_capture_modes"] = captureModes
	e.Metrics["reminders_scope_fallback_entities"] = fallbackEntities
	reads := []map[string]any{}
	queues := []map[string]any{}
	refetches := []map[string]any{}
	for _, event := range trace {
		if event.Stage == "refetch" {
			refetches = append(refetches, map[string]any{"started_ns": event.StartedNS, "finished_ns": event.FinishedNS, "recorded_ns": event.UptimeNS, "recorded_wall_ns": event.WallNS, "source": event.Source, "scope_started_ns": event.ScopeStartedNS, "scope_finished_ns": event.ScopeFinishedNS})
		}
		if event.Stage == "reminders_store_read" {
			reads = append(reads, map[string]any{"started_ns": event.StartedNS, "finished_ns": event.FinishedNS, "started_wall_ns": event.StartedWallNS, "finished_wall_ns": event.FinishedWallNS, "mode": event.Mode, "decoded_rows": event.DecodedRows, "duration_ms": event.DurationMS})
		}
		if event.Stage == "reminders_store_queue" {
			queues = append(queues, map[string]any{"started_ns": event.StartedNS, "finished_ns": event.FinishedNS})
		}
	}
	e.Metrics["reminders_store_reads"] = reads
	e.Metrics["eventkit_refetch_windows"] = refetches
	e.Metrics["reminders_store_queues"] = queues
	if len(effects) == 0 {
		stages := []map[string]any{}
		for _, event := range trace {
			if event.Source == "reminders" || strings.HasPrefix(event.Stage, "reminders_") {
				stages = append(stages, map[string]any{"stage": event.Stage, "uptime_ns": event.UptimeNS, "wall_ns": event.WallNS, "started_ns": event.StartedNS, "finished_ns": event.FinishedNS, "records": len(event.Keys)})
			}
		}
		e.Metrics["reminders_startup_stages"] = stages
	}
	e.Latency["calendar_chunk_refetch"] = stats(calendarRefetch, 0)
	nativeStartupEvidence(w, e, trace)
	refreshes := []nativeTrace{}
	for _, event := range trace {
		if event.Stage == "reminders_store_refresh" {
			refreshes = append(refreshes, event)
		}
	}
	if len(refreshes) > 0 {
		if e.Metrics == nil {
			e.Metrics = map[string]any{}
		}
		e.Metrics["reminders_store_refresh_histogram"] = refreshes[len(refreshes)-1].Histogram
		samples := make([]map[string]any, 0, len(refreshes))
		warm := []float64{}
		for index, event := range refreshes {
			samples = append(samples, map[string]any{"index": index, "started_ns": event.StartedNS, "finished_ns": event.FinishedNS, "duration_ms": event.DurationMS})
			if index > 0 {
				warm = append(warm, event.DurationMS)
			}
		}
		e.Metrics["reminders_store_refresh_samples"] = samples
		e.Latency["reminders_store_refresh_warm"] = stats(warm, 0)
	}

	values := map[string][]float64{}
	sample := func(name string, from, through int64) {
		if from > 0 && through >= from {
			values[name] = append(values[name], float64(through-from)/float64(time.Millisecond))
		}
	}
	for _, raw := range effects {
		effect := raw.(map[string]any)
		collection, _ := effect["collection"].(string)
		if collection == "" {
			collection = "reminders.reminder"
		}
		key, _ := effect["key"].(string)
		title, _ := effect["title"].(string)
		save, _ := effect["effect_uptime_ns"].(json.Number).Int64()
		wall := float64(0)
		if value, ok := effect["effect_wall_ns"].(json.Number); ok {
			wall, _ = value.Float64()
		}
		row := NativeTiming{SaveNS: save, SaveWallNS: wall}
		checkpoint := ""
		for _, event := range trace {
			if (event.Stage == "reminders_database_hint" && collection == "reminders.reminder" || event.Stage == "calendar_database_hint" && collection != "reminders.reminder") && event.DeliveredNS >= save && row.DatabaseHintNS == 0 {
				row.DatabaseHintNS = event.DeliveredNS
			}
			if event.Stage == "notification" && event.UptimeNS >= save && row.NotificationNS == 0 {
				row.NotificationNS = event.UptimeNS
				if event.DeliveredNS > 0 {
					row.NotificationNS = event.DeliveredNS
				}
			}
			if event.UptimeNS < save {
				continue
			}
			matches := false
			for _, candidate := range event.Keys {
				if candidate == key {
					matches = true
				}
			}
			if !matches {
				continue
			}
			switch event.Stage {
			case "refetch":
				if row.RefetchFinishedNS == 0 {
					row.CaptureMode = event.Mode
					row.ScopeStartedNS = event.ScopeStartedNS
					row.ScopeFinishedNS = event.ScopeFinishedNS
					row.TriggerNotificationNS = event.NotificationNS
					row.DebounceNS = event.DebounceNS
					row.RefetchStartedNS = event.StartedNS
					row.RefetchFinishedNS = event.FinishedNS
				}
			case "reminders_store_refresh":
				if row.StoreRefreshFinishedNS == 0 {
					row.StoreRefreshStartedNS = event.StartedNS
					row.StoreRefreshFinishedNS = event.FinishedNS
					row.StoreRefreshMS = event.DurationMS
				}
			case "capture_sent":
				if row.CaptureSentNS == 0 {
					row.CaptureSentNS = event.UptimeNS
					row.CaptureWallNS = event.WallNS
					checkpoint = event.Checkpoint
				}
			case "committed_ack":
				if event.Checkpoint == checkpoint && row.CommitAckNS == 0 {
					row.CommitAckNS = event.UptimeNS
				}
			}
		}
		for _, event := range commits {
			if event.Checkpoint == checkpoint {
				row.GatewayCommitWallNS = event.WallNS
				row.CommitStartedWallNS = event.CommitStartedWallNS
				break
			}
		}
		if seen, ok := observers[0].firstObservedExact(title, collection); ok {
			row.GatewayVisibleNS = seen.UptimeNS
			row.GatewayVisibleWallNS = seen.At.UnixNano()
		}
		if seen, ok := observers[1].firstObservedExact(title, collection); ok {
			row.ReplicaVisibleNS = seen.UptimeNS
			row.ReplicaVisibleWallNS = seen.At.UnixNano()
		}
		sample("native_save→capture_trigger", row.SaveNS, row.TriggerNotificationNS)
		sample("native_save→next_notification_delivery", row.SaveNS, row.NotificationNS)
		sample("native_save→database_hint_delivery", row.SaveNS, row.DatabaseHintNS)
		sample("trigger_notification→debounce", row.TriggerNotificationNS, row.DebounceNS)
		sample("debounce→scope_start", row.DebounceNS, row.ScopeStartedNS)
		sample("incremental_scope_read", row.ScopeStartedNS, row.ScopeFinishedNS)
		sample("debounce→refetch_start", row.DebounceNS, row.RefetchStartedNS)
		sample("refetch_duration", row.RefetchStartedNS, row.RefetchFinishedNS)
		sample("refetch_finished→capture_sent", row.RefetchFinishedNS, row.CaptureSentNS)
		sample("capture_sent→commit_ack", row.CaptureSentNS, row.CommitAckNS)
		sample("capture_sent→gateway_commit_start", int64(row.CaptureWallNS), int64(row.CommitStartedWallNS))
		sample("gateway_commit_duration", int64(row.CommitStartedWallNS), int64(row.GatewayCommitWallNS))
		sample("capture_sent→gateway_commit", int64(row.CaptureWallNS), int64(row.GatewayCommitWallNS))
		sample("gateway_commit→replica_visible", int64(row.GatewayCommitWallNS), row.ReplicaVisibleWallNS)
		sample("trigger_notification→replica_visible", row.TriggerNotificationNS, row.ReplicaVisibleNS)
		e.NativeTimings = append(e.NativeTimings, row)
	}
	for name, samples := range values {
		summary := stats(samples, 0)
		summary.Unpaired = len(effects) - len(samples)
		e.Latency[name] = summary
	}
}

func nativeConditions() map[string]string {
	out := map[string]string{}
	for name, args := range map[string][]string{"cpu": {"sh", "-c", "top -l 1 -n 0 | sed -n '/CPU usage:/p'"}, "uptime": {"uptime"}, "swap": {"sysctl", "vm.swapusage"}, "memory_pressure": {"memory_pressure", "-Q"}, "heavy_slots": {"sh", "-c", "for slot in /tmp/apple-bridge-heavy-*.lock/pid; do cat \"$slot\"; done"}} {
		bytes, err := exec.Command(args[0], args[1:]...).Output()
		if err == nil {
			out[name] = string(bytes)
		}
	}
	return out
}
