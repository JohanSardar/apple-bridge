package scenarios

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"os/exec"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func runDateFixtures(ctx context.Context, s Scenario, level string) Evidence {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e := Evidence{GitSHA: strings.TrimSpace(string(sha)), RunID: contracts.NewID("date"), Scenario: s.ID, Level: level, At: time.Now().UTC(), Passed: true, Assertions: []Assertion{}}
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
	calendar := s.ID == "CAL-DST"
	collection := "reminders.reminder"
	if calendar {
		collection = "calendar.event"
		add("calendar", w.ControlAction("/calendar", map[string]any{"action": "calendar", "key": "test-calendar"}) == nil)
	}
	if !calendar {
		add("reminder_list", w.ControlAction("/external-change", map[string]any{"collection": "reminders.list", "key": "date-list", "data": map[string]any{"title": "Bridge Test Dates", "writable": true}}) == nil)
	}
	type fixture struct {
		name, start, end, zone, frequency string
		allDay                            bool
		starts                            []string
	}
	cases := []fixture{
		{"spring_timed", "2026-03-08T01:30:00-05:00", "2026-03-08T03:30:00-04:00", "America/New_York", "", false, []string{"2026-03-08T01:30:00-05:00"}},
		{"fall_timed", "2026-11-01T01:30:00-04:00", "2026-11-01T01:30:00-05:00", "America/New_York", "", false, []string{"2026-11-01T01:30:00-04:00"}},
		{"spring_floating", "2026-03-08", "2026-03-09", "", "", true, []string{"2026-03-08"}},
		{"fall_floating", "2026-11-01", "2026-11-02", "", "", true, []string{"2026-11-01"}},
		{"spring_daily", "2026-03-07T09:00:00-05:00", "2026-03-07T10:00:00-05:00", "America/New_York", "daily", false, []string{"2026-03-07T14:00:00Z", "2026-03-08T13:00:00Z", "2026-03-09T13:00:00Z"}},
		{"fall_daily", "2026-10-31T09:00:00-04:00", "2026-10-31T10:00:00-04:00", "America/New_York", "daily", false, []string{"2026-10-31T13:00:00Z", "2026-11-01T14:00:00Z", "2026-11-02T14:00:00Z"}},
		{"spring_allday_daily", "2026-03-07", "2026-03-08", "America/New_York", "daily", true, []string{"2026-03-07", "2026-03-08", "2026-03-09"}},
		{"fall_allday_daily", "2026-10-31", "2026-11-01", "America/New_York", "daily", true, []string{"2026-10-31", "2026-11-01", "2026-11-02"}},
		{"utc_daily", "2026-03-07T14:00:00Z", "2026-03-07T15:00:00Z", "UTC", "daily", false, []string{"2026-03-07T14:00:00Z", "2026-03-08T14:00:00Z", "2026-03-09T14:00:00Z"}},
		{"month_end", "2026-01-31", "2026-02-01", "", "monthly", true, []string{"2026-01-31", "2026-03-31", "2026-05-31"}},
	}
	for _, f := range cases {
		data := map[string]any{"title": "Bridge Test " + f.name, "start": f.start, "end": f.end, "all_day": f.allDay, "time_zone": f.zone, "calendar_id": rmRef("calendar.calendar", "test-calendar")}
		if f.frequency != "" {
			data["recurrence"] = []any{map[string]any{"frequency": f.frequency, "interval": 1, "count": 3}}
		}
		if calendar {
			err = w.ControlAction("/calendar", map[string]any{"action": "create", "key": f.name, "data": data})
		} else {
			alarm := f.start
			if f.allDay {
				alarm = f.start + "T13:00:00Z"
			}
			data = map[string]any{"title": "Bridge Test " + f.name, "due": f.start, "due_all_day": f.allDay, "completed": false, "list_id": rmRef("reminders.list", "date-list"), "alarms": []any{map[string]any{"absolute_date": alarm, "relative_offset": 0, "proximity": 0}, map[string]any{"absolute_date": nil, "relative_offset": -1800, "proximity": 0}}}
			err = w.ControlAction("/external-change", map[string]any{"collection": collection, "key": f.name, "data": data})
		}
		add(f.name+"_fixture", err == nil)
		want := 1
		if calendar {
			want = len(f.starts)
		}
		add(f.name+"_readback", w.Await(5*time.Second, func() bool {
			records, readErr := w.Records(w.Gateway)
			if readErr != nil {
				return false
			}
			found := map[string]bool{}
			matched := 0
			for _, r := range records {
				if r.Collection != collection || recordTitle(r) != "Bridge Test "+f.name {
					continue
				}
				matched++
				var got map[string]any
				if json.Unmarshal(r.Data, &got) != nil {
					return false
				}
				if calendar {
					if got["all_day"] != f.allDay || got["time_zone"] != f.zone {
						return false
					}
					start := fmt.Sprint(got["start"])
					for _, expected := range f.starts {
						if start == expected {
							found[expected] = true
						}
					}
					if f.frequency == "" && got["end"] != f.end {
						return false
					}
				} else {
					if got["due"] != f.start || got["due_all_day"] != f.allDay || !sameDateFixtureJSON(got["alarms"], data["alarms"]) {
						return false
					}
					found[f.start] = true
				}
			}
			return len(found) == want && matched == want
		}) == nil)
	}
	add("replica_converged", w.Await(5*time.Second, func() bool { return convergence(w) }) == nil)
	return e
}

func sameDateFixtureJSON(a, b any) bool {
	x, _ := json.Marshal(a)
	y, _ := json.Marshal(b)
	return bytes.Equal(x, y)
}
