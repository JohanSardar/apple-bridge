package scenarios

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"

	"github.com/JohanSardar/apple-bridge/lab/world"
)

type contactsTrace struct {
	Keys                []string `json:"keys,omitempty"`
	NotificationNS      int64    `json:"notification_ns,omitempty"`
	Operation           string   `json:"operation,omitempty"`
	Stage               string   `json:"stage"`
	ScanID              string   `json:"scan_id,omitempty"`
	Source              string   `json:"source,omitempty"`
	Checkpoint          string   `json:"checkpoint,omitempty"`
	StartedNS           int64    `json:"started_ns,omitempty"`
	PreparedNS          int64    `json:"prepared_ns,omitempty"`
	FinishedNS          int64    `json:"finished_ns,omitempty"`
	UptimeNS            int64    `json:"uptime_ns"`
	WallNS              float64  `json:"wall_ns,omitempty"`
	CommitStartedWallNS float64  `json:"commit_started_wall_ns,omitempty"`
	TokenChanged        bool     `json:"token_changed,omitempty"`
	FullScan            bool     `json:"full_scan,omitempty"`
	Empty               bool     `json:"empty,omitempty"`
}

func readContactsTrace(w *world.World, e *Evidence) []contactsTrace {
	events := []contactsTrace{}
	dir := filepath.Join("lab/evidence/private/contacts", e.RunID)
	os.MkdirAll(dir, 0700)
	for _, input := range []struct{ path, name string }{
		{filepath.Join(w.Gateway.Home, "helper", "capture-trace.jsonl"), "helper.jsonl"},
		{filepath.Join(w.Gateway.Home, "capture-trace.jsonl"), "gateway.jsonl"},
		{filepath.Join(w.Gateway.Home, "helper", "media-work-trace.jsonl"), "media.jsonl"},
		{filepath.Join(w.Dir, "gateway.log"), "gateway.log"},
	} {
		raw, err := os.ReadFile(input.path)
		if err != nil {
			continue
		}
		os.WriteFile(filepath.Join(dir, input.name), raw, 0600)
		for _, line := range strings.Split(string(raw), "\n") {
			var event contactsTrace
			if json.Unmarshal([]byte(line), &event) == nil {
				events = append(events, event)
			}
		}
	}
	return events
}
