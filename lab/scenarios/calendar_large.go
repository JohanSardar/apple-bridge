package scenarios

import (
	"context"
	"os/exec"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func runCalendarLargeCapture(ctx context.Context, s Scenario, level string) Evidence {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e := Evidence{GitSHA: strings.TrimSpace(string(sha)), RunID: contracts.NewID("cal"), Scenario: s.ID, Level: level, At: time.Now().UTC(), Conditions: calendarHostConditions(), Hosts: []string{"air"}, Latency: map[string]Latency{}, Assertions: []Assertion{}, Passed: true}
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
	w.Control.Timeout = 45 * time.Second
	tag := "[ab-test " + e.RunID + "]"
	err = w.ControlAction("/calendar", map[string]any{"action": "calendar", "key": "test-calendar"})
	add("calendar_created", err == nil)
	err = w.ControlAction("/calendar", map[string]any{"action": "create_many", "key": e.RunID, "count": 3000, "data": map[string]any{"title": tag, "start": "2026-11-01T10:00:00Z", "end": "2026-11-01T11:00:00Z"}})
	add("single_change_imports_3000_events", err == nil)
	var target contracts.Record
	add("large_capture_converges", w.Await(45*time.Second, func() bool {
		for _, endpoint := range append([]world.Endpoint{w.Gateway}, w.Replicas...) {
			records, err := w.Records(endpoint)
			count := 0
			for _, record := range records {
				if record.Collection == "calendar.event" && strings.HasPrefix(recordTitle(record), tag) {
					count++
					target = record
				}
			}
			if err != nil || count != 3000 {
				return false
			}
		}
		return true
	}) == nil)
	if target.ID == "" {
		return e
	}
	var command contracts.CommandRecord
	client := scripted.Client{Endpoint: w.Gateway, Protocol: "rest"}
	err = client.Call(ctx, "calendar.update_event", contracts.Command{ID: contracts.NewCommandID(), Args: mustJSON(map[string]any{"id": target.ID, "span": "future", "title": tag + " continued"}), ExpectedVersion: target.Version, WaitMS: 15000}, &command)
	add("source_captures_after_large_change", err == nil && command.Disposition == "locally_observed")
	add("following_change_reaches_replica", w.Await(15*time.Second, func() bool {
		for _, endpoint := range append([]world.Endpoint{w.Gateway}, w.Replicas...) {
			records, _ := w.Records(endpoint)
			found := false
			for _, record := range records {
				found = found || recordTitle(record) == tag+" continued"
			}
			if !found {
				return false
			}
		}
		return true
	}) == nil)
	return e
}
