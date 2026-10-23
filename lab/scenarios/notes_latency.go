package scenarios

import (
	"encoding/json"
	"os"
	"path/filepath"
	"slices"
	"strings"

	"github.com/JohanSardar/apple-bridge/lab/world"
)

type notesVisibilitySample struct {
	GatewayContent int64 `json:"gateway_content_ns"`
	ReplicaContent int64 `json:"replica_content_ns"`
	Save           int64 `json:"native_save_ns"`
	Gateway        int64 `json:"gateway_first_ns"`
	Replica        int64 `json:"replica_first_ns"`
}

type notesTimingEvent struct {
	Stage      string   `json:"stage"`
	Scan       string   `json:"scan_id"`
	Source     string   `json:"source"`
	Keys       []string `json:"keys"`
	Checkpoint string   `json:"checkpoint"`
	At         int64    `json:"uptime_ns"`
	Wall       float64  `json:"wall_ns"`
}

func notesStageLatencies(w *world.World, e *Evidence, key string, samples []notesVisibilitySample) {
	events := []notesTimingEvent{}
	for _, path := range []string{filepath.Join(w.Gateway.Home, "helper", "capture-trace.jsonl"), filepath.Join(w.Gateway.Home, "capture-trace.jsonl")} {
		raw, _ := os.ReadFile(path)
		for _, line := range strings.Split(string(raw), "\n") {
			var event notesTimingEvent
			if json.Unmarshal([]byte(line), &event) == nil {
				events = append(events, event)
			}
		}
	}
	scans := map[string]map[string]notesTimingEvent{}
	commits := map[string]notesTimingEvent{}
	for _, event := range events {
		if event.Stage == "gateway_commit" {
			commits[event.Checkpoint] = event
		}
		if event.Scan != "" && (event.Source == "notes" || strings.HasPrefix(event.Stage, "notes_")) {
			if scans[event.Scan] == nil {
				scans[event.Scan] = map[string]notesTimingEvent{}
			}
			scans[event.Scan][event.Stage] = event
		}
	}
	values := map[string][]float64{}
	add := func(name string, from, through int64) {
		if from > 0 && through >= from {
			values[name] = append(values[name], float64(through-from)/1e6)
		}
	}
	for _, sample := range samples {
		var selected map[string]notesTimingEvent
		for _, scan := range scans {
			ready := scan["notes_capture_ready"]
			if ready.At >= sample.Save && ready.At <= sample.Gateway && slices.Contains(ready.Keys, key) && (selected == nil || ready.At < selected["notes_capture_ready"].At) {
				selected = scan
			}
		}
		if selected == nil {
			continue
		}
		start := selected["notes_scan_started"].At
		read := selected["notes_store_read"].At
		hydrated := selected["notes_metadata_projected"].At
		ready := selected["notes_capture_ready"].At
		var wal, hint int64
		for _, event := range events {
			if event.At >= sample.Save-50_000_000 && event.At <= start && event.Stage == "notes_wal_changed" && (wal == 0 || event.At < wal) {
				wal = event.At
			}
			if event.At >= sample.Save && event.At <= start && event.Stage == "notes_fsevent_hint" && (hint == 0 || event.At < hint) {
				hint = event.At
			}
		}
		add("notes_native_save_to_wal_hint_ms", sample.Save, wal)
		add("notes_wal_to_fsevent_ms", wal, hint)
		add("notes_fsevent_to_scan_ms", hint, start)
		add("notes_native_save_to_scan_ms", sample.Save, start)
		add("notes_store_scan_ms", start, read)
		add("notes_metadata_projection_ms", read, hydrated)
		add("notes_hydration_ms", selected["notes_hydration_started"].At, selected["notes_hydration_finished"].At)
		add("notes_compare_ms", hydrated, ready)
		sent := selected["capture_sent"]
		commit, ok := commits[sent.Checkpoint]
		if ok && sent.At > 0 {
			committed := sent.At + int64(commit.Wall-sent.Wall)
			add("notes_ready_to_commit_ms", ready, committed)
			add("notes_commit_to_gateway_visible_ms", committed, sample.Gateway)
			add("notes_commit_to_replica_visible_ms", committed, sample.Replica)
		}
	}
	for name, values := range values {
		e.Latency[name] = stats(values, 0)
	}
	e.Metrics["first_visibility_samples"] = samples
}

func notesImportDiagnostics(w *world.World, e *Evidence, title string) {
	if e.Metrics == nil {
		e.Metrics = map[string]any{}
	}
	counts := map[string]int{}
	for _, path := range []string{filepath.Join(w.Gateway.Home, "helper", "capture-trace.jsonl"), filepath.Join(w.Gateway.Home, "capture-trace.jsonl")} {
		raw, _ := os.ReadFile(path)
		for _, line := range strings.Split(string(raw), "\n") {
			var event notesTimingEvent
			if json.Unmarshal([]byte(line), &event) == nil && (event.Source == "notes" || strings.HasPrefix(event.Stage, "notes_")) {
				counts[event.Stage]++
			}
		}
	}
	e.Metrics["notes_import_trace_counts"] = counts
	for index, endpoint := range []world.Endpoint{w.Gateway, w.Replicas[0]} {
		label := []string{"gateway", "replica"}[index]
		rows, err := w.Records(endpoint)
		found, total := false, 0
		for _, record := range rows {
			if record.Collection == "notes.note" && !record.Deleted {
				total++
				found = found || recordTitle(record) == title
			}
		}
		e.Metrics[label+"_notes_import_read_ok"] = err == nil
		e.Metrics[label+"_notes_import_count"] = total
		e.Metrics[label+"_tagged_note_imported"] = found
	}
}
