package scenarios

import (
	"context"
	"encoding/json"
	"os/exec"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func runCalendarVerification(ctx context.Context, s Scenario, level string) Evidence {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e := Evidence{GitSHA: strings.TrimSpace(string(sha)), RunID: contracts.NewID("cal"), Scenario: s.ID, Level: level, At: time.Now().UTC(), Conditions: calendarHostConditions(), Hosts: []string{"air"}, Latency: map[string]Latency{}, Passed: true}
	add := func(name string, ok bool) {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok})
		e.Passed = e.Passed && ok
	}
	w, err := world.New(ctx, level, 1, 0)
	add("world_start", err == nil)
	if err != nil {
		return e
	}
	defer w.Close()
	control := func(action string, values map[string]any) bool {
		values["action"] = action
		return w.ControlAction("/calendar", values) == nil
	}
	add("calendar_created", control("calendar", map[string]any{"key": "test-calendar"}))
	add("verification_clock_started", control("clock", map[string]any{"now": "2026-11-01T00:00:00Z"}))
	title := "[ab-test " + e.RunID + "] verification"
	add("event_created", control("create", map[string]any{"key": "verification-event", "data": map[string]any{"title": title, "start": "2026-11-01T10:00:00Z", "end": "2026-11-01T11:00:00Z"}}))
	visible := func(status float64) bool {
		for _, endpoint := range append([]world.Endpoint{w.Gateway}, w.Replicas...) {
			records, err := w.Records(endpoint)
			found := false
			for _, record := range records {
				var data map[string]any
				_ = json.Unmarshal(record.Data, &data)
				if data["title"] == title {
					attendees, _ := data["attendees"].([]any)
					found = status == 0 && len(attendees) == 0
					if len(attendees) == 1 {
						attendee, _ := attendees[0].(map[string]any)
						found = attendee["status"] == status
					}
				}
			}
			if err != nil || !found {
				return false
			}
		}
		return true
	}
	add("initial_status_visible", w.Await(5*time.Second, func() bool { return visible(0) }) == nil)
	add("server_status_changes_with_injected_stamp_match", control("server_update", map[string]any{"key": "verification-event", "force_stamp_match": true, "data": map[string]any{"attendees": []any{map[string]any{"name": "Bridge Test", "url": "mailto:guest@example.com", "status": 2, "role": 1, "type": 1}}}}))
	add("ordinary_scan_reuses_unchanged_stamp", control("clock", map[string]any{"now": "2026-11-01T00:14:59Z"}) && visible(0))
	add("periodic_verification_triggered", control("clock", map[string]any{"now": "2026-11-01T00:15:00Z"}))
	add("periodic_verification_rebuilds_unstamped_status", w.Await(3*time.Second, func() bool { return visible(2) }) == nil)
	add("periodic_verification_matches_independent_truth", w.Await(3*time.Second, func() bool { return nativeConvergence(w) }) == nil)
	return e
}
