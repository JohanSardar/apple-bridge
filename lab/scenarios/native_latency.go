package scenarios

import (
	"bufio"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"time"

	"github.com/JohanSardar/apple-bridge/lab/world"
)

type nativeTrace struct {
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
	GatewayVisibleWallNS  int64   `json:"gateway_visible_wall_ns"`
	ReplicaVisibleWallNS  int64   `json:"replica_visible_wall_ns"`
	CommitStartedWallNS   float64 `json:"commit_started_wall_ns"`
	TriggerNotificationNS int64   `json:"trigger_notification_ns"`
	SaveNS                int64   `json:"save_ns"`
	NotificationNS        int64   `json:"notification_ns"`
	DebounceNS            int64   `json:"debounce_ns"`
	RefetchStartedNS      int64   `json:"refetch_started_ns"`
	RefetchFinishedNS     int64   `json:"refetch_finished_ns"`
	CaptureSentNS         int64   `json:"capture_sent_ns"`
	CommitAckNS           int64   `json:"commit_ack_ns"`
	SaveWallNS            float64 `json:"save_wall_ns"`
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
	values := map[string][]float64{}
	sample := func(name string, from, through int64) {
		if from > 0 && through >= from {
			values[name] = append(values[name], float64(through-from)/float64(time.Millisecond))
		}
	}
	for _, raw := range effects {
		effect := raw.(map[string]any)
		key, _ := effect["key"].(string)
		title, _ := effect["title"].(string)
		save, _ := effect["effect_uptime_ns"].(json.Number).Int64()
		wall, _ := effect["effect_wall_ns"].(json.Number).Float64()
		row := NativeTiming{SaveNS: save, SaveWallNS: wall}
		checkpoint := ""
		for _, event := range trace {
			if event.Stage == "notification" && event.UptimeNS >= save && row.NotificationNS == 0 {
				row.NotificationNS = event.UptimeNS
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
					row.TriggerNotificationNS = event.NotificationNS
					row.DebounceNS = event.DebounceNS
					row.RefetchStartedNS = event.StartedNS
					row.RefetchFinishedNS = event.FinishedNS
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
		if seen, ok := observers[0].observedExact(title); ok {
			row.GatewayVisibleNS = seen.UptimeNS
			row.GatewayVisibleWallNS = seen.At.UnixNano()
		}
		if seen, ok := observers[1].observedExact(title); ok {
			row.ReplicaVisibleNS = seen.UptimeNS
			row.ReplicaVisibleWallNS = seen.At.UnixNano()
		}
		sample("native_save→next_notification_delivery", row.SaveNS, row.NotificationNS)
		sample("trigger_notification→debounce", row.TriggerNotificationNS, row.DebounceNS)
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
	for name, args := range map[string][]string{"cpu": {"sh", "-c", "top -l 1 -n 0 | sed -n '/CPU usage:/p'"}, "uptime": {"uptime"}, "memory_pressure": {"memory_pressure", "-Q"}, "heavy_slots": {"sh", "-c", "for slot in /tmp/apple-bridge-heavy-*.lock/pid; do cat \"$slot\"; done"}} {
		bytes, err := exec.Command(args[0], args[1:]...).Output()
		if err == nil {
			out[name] = string(bytes)
		}
	}
	return out
}
