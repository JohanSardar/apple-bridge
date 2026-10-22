package scenarios

import (
	"context"
	"encoding/json"
	"os/exec"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func runCalendarStoreReal(ctx context.Context, scenario Scenario, level string) (e Evidence) {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e = Evidence{RunID: contracts.NewID("cal"), Scenario: scenario.ID, Level: level, GitSHA: strings.TrimSpace(string(sha)), At: time.Now().UTC(), Passed: true, Conditions: calendarHostConditions(), Latency: map[string]Latency{}, Metrics: map[string]any{}, Hosts: []string{"air"}}
	add := func(name string, passed bool) {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: passed})
		e.Passed = e.Passed && passed
	}
	tag := "[ab-test " + e.RunID + "]"
	w, err := world.NewRealWithSetup(ctx, level, 1, world.NativeSetup{})
	add("world_start", err == nil)
	if err != nil {
		return e
	}
	defer w.Close()
	defer func() {
		_, cleanup := w.RecoverTruth("fixture", map[string]any{"source": "calendar", "action": "cleanup", "tag": tag})
		if cleanup == nil {
			_, cleanup = w.RecoverTruth("fixture", map[string]any{"source": "calendar", "action": "cleanup_run_calendar", "tag": tag})
		}
		audit, auditErr := w.RecoverTruth("observe", map[string]any{"source": "calendar", "action": "audit", "tag": tag})
		remaining, known := audit["records"].([]any)
		add("tagged_fixture_cleanup_audit", cleanup == nil && auditErr == nil && known && len(remaining) == 0)
	}()
	_, err = w.Truth("fixture", map[string]any{"source": "calendar", "action": "create", "tag": tag, "title": tag + " store fixture", "start": "2026-11-01T15:00:00Z", "end": "2026-11-01T16:00:00Z", "time_zone": "America/New_York"})
	add("tagged_native_fixture", err == nil)
	if err != nil {
		return e
	}
	var record contracts.Record
	visible := w.Await(90*time.Second, func() bool {
		records, readErr := w.Records(w.Gateway)
		if readErr != nil {
			return false
		}
		for _, item := range records {
			if item.Collection == "calendar.event" && strings.HasPrefix(recordTitle(item), tag) {
				var fields map[string]any
				json.Unmarshal(item.Data, &fields)
				if fields["store_status"] != nil {
					record = item
					return true
				}
			}
		}
		return false
	}) == nil
	add("native_fixture_store_capture", visible)
	if !visible {
		return e
	}
	truth, truthErr := w.Truth("observe", map[string]any{"source": "calendar", "action": "store_counts", "tag": tag})
	if truthErr != nil || truth["status"] != "verified" {
		e.Assertions = append(e.Assertions, Assertion{Name: "independent_calendar_store_counts", Passed: false, Status: "unverified", Detail: "AppleTruth Calendar store access unavailable"})
	} else {
		count := func(key string, expected int64) bool {
			value, known := truth[key].(json.Number)
			actual, parseErr := value.Int64()
			return known && parseErr == nil && actual == expected
		}
		add("independent_calendar_store_counts", count("event_count", 1) && count("store_count", 1) && count("attachment_count", 0))
		e.Metrics["native_event_count"] = truth["event_count"]
		e.Metrics["native_store_count"] = truth["store_count"]
		e.Metrics["native_attachment_count"] = truth["attachment_count"]
		e.Metrics["store_identity_columns"] = truth["identifier_columns"]
		e.Metrics["store_external_identity_columns"] = truth["external_identifier_columns"]
		e.Metrics["calendar_identity_matches"] = truth["calendar_identity_matches"]
	}
	var fields map[string]any
	json.Unmarshal(record.Data, &fields)
	e.Metrics["store_status"] = fields["store_status"]
	e.Metrics["store_identifier_match"] = fields["store_identifier_match"]
	if fields["store_status"] == "unsupported" {
		e.Assertions = append(e.Assertions, Assertion{Name: "native_store_identity", Passed: false, Status: "unverified", Detail: "Calendar store identity could not be proven; store fields remain hidden"})
	} else {
		add("native_store_identity", fields["store_identifier_match"] != nil)
	}
	client := scripted.Client{Endpoint: w.Gateway, Protocol: "rest"}
	var page calendarStorePage
	err = client.Call(ctx, "calendar.range", map[string]any{"start": "2026-11-01T00:00:00Z", "end": "2026-11-02T00:00:00Z", "time_zone": "America/New_York"}, &page)
	found := false
	for _, item := range page.Events {
		found = found || item.Event.ID == record.ID
	}
	add("native_event_in_range_read_model", err == nil && found)
	return e
}
