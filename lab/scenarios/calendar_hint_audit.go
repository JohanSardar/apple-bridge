package scenarios

import (
	"bytes"
	"encoding/json"
	"os"
	"path/filepath"
	"time"

	"github.com/JohanSardar/apple-bridge/lab/world"
)

func calendarReminderHintAudit(w *world.World, e *Evidence, started, finished time.Time) bool {
	raw, err := os.ReadFile(filepath.Join(w.Gateway.Home, "helper", "capture-trace.jsonl"))
	if err != nil {
		return false
	}
	calendarScanWaitEvidence(e, raw)
	directory := filepath.Join(os.Getenv("HOME"), "Library/Application Support/AppleBridgeDev/cal-hint-traces")
	if os.MkdirAll(directory, 0700) != nil || os.WriteFile(filepath.Join(directory, e.RunID+".jsonl"), raw, 0600) != nil {
		return false
	}
	type row struct {
		Stage        string  `json:"stage"`
		Wall         float64 `json:"wall_ns"`
		Notification uint64  `json:"notification_ns"`
		Delivered    uint64  `json:"delivered_ns"`
		Scoped       bool    `json:"scoped"`
		Full         bool    `json:"force_full"`
		Origin       string  `json:"origin"`
	}
	database := map[uint64]bool{}
	rows := []row{}
	lines := bytes.Split(raw, []byte("\n"))
	if !bytes.HasSuffix(raw, []byte("\n")) {
		lines = lines[:len(lines)-1]
	}
	for _, line := range lines {
		if len(line) == 0 {
			continue
		}
		var entry row
		if json.Unmarshal(line, &entry) != nil {
			return false
		}
		if entry.Stage == "calendar_database_hint" {
			database[entry.Delivered] = true
		}
		if entry.Wall >= float64(started.UnixNano()) && entry.Wall <= float64(finished.UnixNano()) {
			rows = append(rows, entry)
		}
	}
	full, uncorroborated, hints := 0, 0, 0
	origins := map[string]int{}
	for _, entry := range rows {
		if entry.Stage == "calendar_database_hint_origin" {
			origins[entry.Origin]++
		}
		if entry.Stage == "calendar_database_hint" {
			hints++
		}
		if entry.Stage == "calendar_scan_started" && !entry.Scoped && !entry.Full {
			full++
			if !database[entry.Notification] {
				uncorroborated++
			}
		}
	}
	if e.Metrics == nil {
		e.Metrics = map[string]any{}
	}
	e.Metrics["reminder_burst_calendar_full_scans"] = full
	e.Metrics["reminder_burst_calendar_database_hints"] = hints
	e.Metrics["reminder_burst_calendar_uncorroborated_full_scans"] = uncorroborated
	e.Metrics["reminder_burst_calendar_hint_origins"] = origins
	return uncorroborated == 0
}
