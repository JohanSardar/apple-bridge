package scenarios

import (
	"context"
	"os/exec"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func runCalendarStampFields(ctx context.Context, s Scenario, level string) Evidence {
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
	control := func(action, key string, data map[string]any) bool {
		return w.ControlAction("/calendar", map[string]any{"action": action, "key": key, "data": data, "force_stamp_match": action == "server_update"}) == nil
	}
	add("calendar_created", control("calendar", "test-calendar", nil))
	add("second_calendar_created", control("calendar", "other-calendar", map[string]any{"title": "Bridge Test second", "writable": true, "account": "test", "type": "local", "color": "#aabbcc"}))
	add("verification_clock_started", w.ControlAction("/calendar", map[string]any{"action": "clock", "now": "2026-11-01T00:00:00Z"}) == nil)
	participant := func(field string, value any) []any {
		p := map[string]any{"name": "Bridge Test", "url": "mailto:guest@example.com", "status": 1, "role": 1, "type": 1}
		p[field] = value
		return []any{p}
	}
	rule := func(field string, value any) []any {
		r := map[string]any{"frequency": "daily", "interval": 1, "count": 3, "end": nil, "first_day_of_week": 1, "days_of_week": []any{}, "days_of_month": []any{}, "days_of_year": []any{}, "weeks_of_year": []any{}, "months": []any{}, "positions": []any{}}
		r[field] = value
		return []any{r}
	}
	changes := []struct {
		name, field string
		value       any
	}{
		{"calendar_id", "calendar_id", map[string]any{"$ref": map[string]string{"collection": "calendar.calendar", "native_key": "other-calendar"}}},
		{"title", "title", "[ab-test stamp] edited"}, {"notes", "notes", "fixture notes"}, {"location", "location", "Bridge Test room"}, {"url", "url", "https://example.com/event"},
		{"start", "start", "2026-11-01T10:30:00Z"}, {"end", "end", "2026-11-01T12:00:00Z"}, {"all_day", "all_day", true}, {"time_zone", "time_zone", "Europe/Paris"},
		{"is_exception", "is_exception", true}, {"availability", "availability", 2}, {"status", "status", 2}, {"series_id", "series_id", "fixture-series"}, {"occurrence_date", "occurrence_date", "2026-11-01T09:00:00Z"},
		{"attendee_name", "attendees", participant("name", "Bridge Test edited")}, {"attendee_url", "attendees", participant("url", "mailto:other@example.com")}, {"attendee_status", "attendees", participant("status", 2)}, {"attendee_role", "attendees", participant("role", 2)}, {"attendee_type", "attendees", participant("type", 2)},
		{"alarm_absolute", "alarms", []any{map[string]any{"absolute_date": "2026-11-01T09:00:00Z", "relative_offset": 0}}}, {"alarm_relative", "alarms", []any{map[string]any{"absolute_date": nil, "relative_offset": -600}}},
		{"recurrence_frequency", "recurrence", rule("frequency", "weekly")}, {"recurrence_interval", "recurrence", rule("interval", 2)}, {"recurrence_count", "recurrence", rule("count", 4)}, {"recurrence_end", "recurrence", rule("end", "2026-12-01T10:00:00Z")}, {"recurrence_first_day", "recurrence", rule("first_day_of_week", 2)},
		{"recurrence_weekday", "recurrence", rule("days_of_week", []any{map[string]any{"day": 2, "week": 1}})}, {"recurrence_monthday", "recurrence", rule("days_of_month", []any{2})}, {"recurrence_yearday", "recurrence", rule("days_of_year", []any{3})}, {"recurrence_yearweek", "recurrence", rule("weeks_of_year", []any{4})}, {"recurrence_month", "recurrence", rule("months", []any{5})}, {"recurrence_position", "recurrence", rule("positions", []any{1})},
	}
	fastFields := map[string]bool{"location": true, "alarm_absolute": true, "alarm_relative": true}
	restartKey := ""
	for index, change := range changes {
		key := contracts.NewID("stamp")
		restartKey = key
		title := "[ab-test " + e.RunID + "] " + change.name
		created := control("create", key, map[string]any{"title": title, "start": "2026-11-01T10:00:00Z", "end": "2026-11-01T11:00:00Z"})
		baseline := control("server_update", key, map[string]any{"attendees": participant("status", 1), "alarms": []any{map[string]any{"absolute_date": nil, "relative_offset": -300}}, "recurrence": rule("frequency", "daily"), "series_id": "baseline-series", "occurrence_date": "2026-11-01T10:00:00Z"})
		at := time.Date(2026, 11, 1, 0, 0, index*1800+900, 0, time.UTC)
		verification := w.ControlAction("/calendar", map[string]any{"action": "clock", "now": at.Format(time.RFC3339)}) == nil
		add(change.name+"_initial", created && baseline && verification && w.Await(3*time.Second, func() bool { return nativeConvergence(w) }) == nil)
		updated := w.ControlAction("/calendar", map[string]any{"action": "server_update", "key": key, "data": map[string]any{change.field: change.value}, "force_stamp_match": !fastFields[change.name]}) == nil
		clock := at.Add(time.Second).Format(time.RFC3339)
		published := w.ControlAction("/calendar", map[string]any{"action": "clock", "now": clock}) == nil
		if fastFields[change.name] {
			add(change.name+"_fast_path_without_modified_date", updated && published && w.Await(time.Second, func() bool { return nativeConvergence(w) }) == nil)
		} else {
			add(change.name+"_retained_before_verification", updated && published && !nativeConvergence(w))
		}
		verified := w.ControlAction("/calendar", map[string]any{"action": "clock", "now": at.Add(900 * time.Second).Format(time.RFC3339)}) == nil
		add(change.name+"_periodic_verification_without_modified_date", verified && w.Await(3*time.Second, func() bool { return nativeConvergence(w) }) == nil)
	}
	restartEdit := control("server_update", restartKey, map[string]any{"notes": "[ab-test " + e.RunID + "] unpublished restart edit"})
	restartAt := time.Date(2026, 11, 1, 0, 0, len(changes)*1800+1, 0, time.UTC)
	restartPulse := w.ControlAction("/calendar", map[string]any{"action": "clock", "now": restartAt.Format(time.RFC3339)}) == nil
	add("metadata_unchanged_edit_unpublished_before_restart", restartEdit && restartPulse && !nativeConvergence(w))
	add("helper_restart_discards_in_memory_stamps", w.RestartHelper() == nil)
	add("restart_full_capture_matches_truth", w.Await(5*time.Second, func() bool { return nativeConvergence(w) }) == nil)
	e.Metrics = map[string]any{"captured_field_edits": len(changes), "verification_interval_seconds": 900, "fast_path_fields": []string{"location", "alarm_absolute", "alarm_relative"}, "forced_stamp_match_for_backstop_fields": true}
	return e
}
