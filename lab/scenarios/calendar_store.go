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

type calendarStorePage struct {
	Events []struct {
		Event   contracts.Record `json:"event"`
		Group   string           `json:"overlap_group"`
		Column  int              `json:"column"`
		Columns int              `json:"columns"`
	} `json:"events"`
	Calendars []contracts.Record `json:"calendars"`
	Cursor    contracts.Cursor   `json:"cursor"`
}

func runCalendarStore(ctx context.Context, scenario Scenario, level string) Evidence {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e := Evidence{RunID: contracts.NewID("run"), Scenario: scenario.ID, Level: level, GitSHA: strings.TrimSpace(string(sha)), At: time.Now().UTC(), Passed: true, Latency: map[string]Latency{}, Hosts: []string{"air"}}
	add := func(name string, passed bool) {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: passed})
		e.Passed = e.Passed && passed
	}
	if level != "sim" {
		return runCalendarStoreReal(ctx, scenario, level)
	}
	w, err := world.New(ctx, level, 1, 0, world.BlobOptions{Scope: "calendar:read,system:read"})
	add("world_start", err == nil)
	if err != nil {
		return e
	}
	defer w.Close()
	for _, key := range []string{"store-work", "store-home"} {
		err = w.ControlAction("/external-change", map[string]any{"collection": "calendar.calendar", "key": key, "data": map[string]any{"title": "Bridge Test " + key, "color": "#007aff", "writable": true, "account": "demo-account", "type": "local"}})
		if err != nil {
			add("calendar_fixture", false)
			return e
		}
	}
	entries := []struct {
		key, start, end, calendar string
		allDay                    bool
	}{
		{"store-a", "2026-03-08T06:30:00Z", "2026-03-08T08:30:00Z", "store-work", false},
		{"store-b", "2026-03-08T07:00:00Z", "2026-03-08T08:00:00Z", "store-work", false},
		{"store-c", "2026-03-08T08:30:00Z", "2026-03-08T09:00:00Z", "store-home", false},
		{"store-floating", "2026-03-08", "2026-03-10", "store-work", true},
		{"store-before", "2026-03-08T04:00:00Z", "2026-03-08T05:00:00Z", "store-work", false},
		{"store-after", "2026-03-09T04:00:00Z", "2026-03-09T05:00:00Z", "store-work", false},
	}
	for _, entry := range entries {
		data := map[string]any{"title": entry.key, "start": entry.start, "end": entry.end, "all_day": entry.allDay, "time_zone": "America/New_York", "calendar_id": map[string]any{"$ref": map[string]string{"collection": "calendar.calendar", "native_key": entry.calendar}}, "travel_time": 1200, "structured_location": map[string]any{"title": "Bridge Test Station", "latitude": 40.7, "longitude": -74.0}, "invitation_status": 2, "attachments": []any{}, "conference_url": "https://example.com/meeting", "notes": ""}
		if entry.key == "store-a" {
			data["notes"] = "train connection"
		}
		err = w.ControlAction("/external-change", map[string]any{"collection": "calendar.event", "key": entry.key, "data": data})
		if err != nil {
			add("event_fixture", false)
			return e
		}
	}
	add("fixtures_converged", w.Await(5*time.Second, func() bool { return convergence(w) }) == nil)
	query := map[string]any{"start": "2026-03-08T00:00:00-05:00", "end": "2026-03-09T00:00:00-04:00", "time_zone": "America/New_York"}
	for endpointIndex, endpoint := range append([]world.Endpoint{w.Gateway}, w.Replicas...) {
		for _, protocol := range []string{"rest", "mcp"} {
			client := scripted.Client{Endpoint: endpoint, Protocol: protocol}
			var page calendarStorePage
			err = client.Call(ctx, "calendar.range", query, &page)
			prefix := protocol + "_" + string(rune('0'+endpointIndex)) + "_"
			add(prefix+"range_available", err == nil)
			if err != nil {
				continue
			}
			add(prefix+"half_open_dst_range", len(page.Events) == 4)
			byTitle := map[string]int{}
			for i, event := range page.Events {
				byTitle[recordTitle(event.Event)] = i
			}
			a, aok := byTitle["store-a"]
			b, bok := byTitle["store-b"]
			c, cok := byTitle["store-c"]
			if aok && bok && cok {
				add(prefix+"overlap_columns", page.Events[a].Columns == 2 && page.Events[b].Columns == 2 && page.Events[a].Column != page.Events[b].Column && page.Events[a].Group == page.Events[b].Group)
				add(prefix+"touching_intervals_separate", page.Events[c].Columns == 1 && page.Events[c].Group != page.Events[a].Group)
			} else {
				add(prefix+"expected_occurrences", false)
			}
			floating, ok := byTitle["store-floating"]
			if ok {
				var data map[string]any
				json.Unmarshal(page.Events[floating].Event.Data, &data)
				add(prefix+"floating_civil_dates", data["start"] == "2026-03-08" && data["end"] == "2026-03-10")
			}
			var fields map[string]any
			if aok {
				json.Unmarshal(page.Events[a].Event.Data, &fields)
			}
			add(prefix+"store_fields_preserved", fields["travel_time"] == float64(1200) && fields["invitation_status"] == float64(2) && fields["structured_location"] != nil)
			calendar := ""
			for _, item := range page.Calendars {
				if recordTitle(item) == "Bridge Test store-work" {
					calendar = item.ID
				}
			}
			filtered := map[string]any{}
			for key, value := range query {
				filtered[key] = value
			}
			filtered["calendars"] = []string{calendar}
			var selected calendarStorePage
			add(prefix+"calendar_filter", calendar != "" && client.Call(ctx, "calendar.range", filtered, &selected) == nil && len(selected.Events) == 3)
			filtered["text"] = "train"
			add(prefix+"notes_search", client.Call(ctx, "calendar.range", filtered, &selected) == nil && len(selected.Events) == 1)
			invalid := map[string]any{"start": query["end"], "end": query["start"]}
			add(prefix+"invalid_range_rejected", client.Call(ctx, "calendar.range", invalid, &selected) != nil)
		}
	}
	client := scripted.Client{Endpoint: w.Gateway, Protocol: "rest"}
	updated := map[string]any{"title": "store-b-updated", "start": "2026-03-08T10:00:00Z", "end": "2026-03-08T11:00:00Z", "all_day": false, "calendar_id": map[string]any{"$ref": map[string]string{"collection": "calendar.calendar", "native_key": "store-work"}}}
	add("cache_update_fixture", w.ControlAction("/external-change", map[string]any{"collection": "calendar.event", "key": "store-b", "data": updated}) == nil)
	add("cached_range_invalidates_after_update", w.Await(5*time.Second, func() bool {
		var page calendarStorePage
		if client.Call(ctx, "calendar.range", query, &page) != nil {
			return false
		}
		found, isolated := false, false
		for _, item := range page.Events {
			found = found || recordTitle(item.Event) == "store-b-updated"
			isolated = isolated || recordTitle(item.Event) == "store-a" && item.Columns == 1
		}
		return found && isolated
	}) == nil)
	add("cache_delete_fixture", w.ControlAction("/external-change", map[string]any{"collection": "calendar.event", "key": "store-b", "delete": true}) == nil)
	add("cached_range_invalidates_after_delete", w.Await(5*time.Second, func() bool {
		var page calendarStorePage
		return client.Call(ctx, "calendar.range", query, &page) == nil && len(page.Events) == 3
	}) == nil)
	for _, endpoint := range append([]world.Endpoint{w.Gateway}, w.Replicas...) {
		token, tokenErr := w.ScopedToken(endpoint, "system:read")
		endpoint.Token = token
		for _, protocol := range []string{"rest", "mcp"} {
			var page calendarStorePage
			err := (scripted.Client{Endpoint: endpoint, Protocol: protocol}).Call(ctx, "calendar.range", query, &page)
			add(protocol+"_warm_cache_respects_scope", tokenErr == nil && err == nil && len(page.Events) == 0 && len(page.Calendars) == 0)
		}
	}
	if e.Passed {
		calendarRangePerf(ctx, w, &e, add)
		native := scenario
		native.ID = "CAL-STORE-NATIVE"
		checks := runCalendarInvalidation(ctx, native, level)
		for _, check := range checks.Assertions {
			add(check.Name, check.Passed)
		}
	}
	return e
}
