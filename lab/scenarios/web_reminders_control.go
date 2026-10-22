package scenarios

import (
	"encoding/json"
	"fmt"
	"strconv"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/demo"
	"github.com/JohanSardar/apple-bridge/lab/simapple"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func webReminderLive(w *world.World, value string) (any, error) {
	index, err := strconv.Atoi(value)
	if err != nil || index < 0 || index >= 20 {
		return nil, fmt.Errorf("invalid live fixture index")
	}
	title, flagged := fmt.Sprintf("Bridge Test live reminder %02d", index), index%2 == 0
	data := map[string]any{
		"title": title, "list_id": rmRef("reminders.list", "demo-rem-travel"), "completed": false,
		"priority": 1, "flagged": flagged, "parent_id": nil, "section_id": nil, "tags": []string{"travel"},
		"due": demo.Epoch.AddDate(0, 0, 3).Format(time.DateOnly), "due_all_day": true, "manual_order": 0,
		"assignment":     map[string]any{"assignee": map[string]any{"id": "demo-me", "is_me": true}},
		"store_coverage": map[string]any{"flagged": map[string]any{"status": "complete"}, "tags": map[string]any{"status": "complete"}, "assignment": map[string]any{"status": "complete"}},
	}
	started := time.Now().UnixMilli()
	err = w.ControlAction("/external-change", map[string]any{"collection": "reminders.reminder", "key": "demo-rem-plan", "data": data})
	return map[string]any{"title": title, "flagged": flagged, "started_at_ms": started}, err
}

func webReminderCapacity(w *world.World) (any, error) {
	records := []contracts.Observation{
		rmObservation("reminders.list", "web-rem-empty", map[string]any{"title": "Bridge Test empty section", "color": "#4b8cf5", "sections": []any{map[string]any{"kind": "store", "id": "web-rem-filled", "title": "Items", "order": 0}, map[string]any{"kind": "store", "id": "web-rem-empty", "title": "Empty", "order": 1}}, "display_order": 1001, "pinned": false}),
		rmObservation("reminders.reminder", "web-rem-only", map[string]any{"title": "Bridge Test only item", "list_id": rmRef("reminders.list", "web-rem-empty"), "completed": false, "flagged": false, "tags": []string{}, "section_id": "web-rem-filled"}),
	}
	for index := 0; index < 100; index++ {
		records = append(records, rmObservation("reminders.reminder", fmt.Sprintf("web-rem-capacity-%04d", index), map[string]any{"title": fmt.Sprintf("Bridge Test capacity %04d", index), "list_id": rmRef("reminders.list", "demo-rem-travel"), "completed": false, "flagged": false, "tags": []string{}, "notes": strings.Repeat("synthetic ", 2400)}))
	}
	err := w.ControlAction("/dataset", simapple.Dataset{Records: records})
	return map[string]any{"ok": err == nil}, err
}

func webReminderPerf(w *world.World) (any, error) {
	records := []contracts.Observation{rmObservation("reminders.list", "web-rem-perf", map[string]any{"title": "Bridge Test 1000 reminders", "color": "#4b8cf5", "sections": []any{}, "display_order": 1000, "pinned": false})}
	for index := 0; index < 1000; index++ {
		records = append(records, rmObservation("reminders.reminder", fmt.Sprintf("web-rem-perf-%04d", index), map[string]any{"title": fmt.Sprintf("Bridge Test Item %04d", index), "list_id": rmRef("reminders.list", "web-rem-perf"), "completed": false, "priority": 0, "flagged": false, "tags": []string{}, "manual_order": index}))
	}
	err := w.ControlAction("/dataset", simapple.Dataset{Records: records})
	return map[string]any{"ok": err == nil}, err
}

func webReminderDraftRace(w *world.World) (any, error) {
	for _, record := range demo.Generate(demo.Options{Seed: 1}).Records {
		if record.NativeKey != "demo-rem-plants" {
			continue
		}
		var data map[string]any
		if err := json.Unmarshal(record.Data, &data); err != nil {
			return nil, err
		}
		data["title"] = "[ab-test] changed while editing"
		err := w.ControlAction("/external-change", map[string]any{"collection": "reminders.reminder", "key": record.NativeKey, "data": data})
		return map[string]any{"ok": err == nil}, err
	}
	return nil, fmt.Errorf("synthetic_reminder_fixture_missing")
}

func webReminderDetails(w *world.World) (any, error) {
	records := []contracts.Observation{}
	for index, title := range []string{"[ab-test] Custom schedule", "[ab-test] Unsupported schedule"} {
		data := map[string]any{"title": title, "list_id": rmRef("reminders.list", "demo-rem-travel"), "completed": false, "priority": 0, "flagged": false, "tags": []string{}, "notes": "[ab-test] A schedule with a location", "due": "2028-01-02T14:00:00Z", "due_all_day": false}
		if index == 0 {
			data["recurrence"] = []any{map[string]any{"frequency": "weekly", "interval": 2, "count": 8, "end": nil, "days_of_week": []any{map[string]any{"day": 2, "week": 0}, map[string]any{"day": 4, "week": 0}}, "days_of_month": []int{}, "days_of_year": []int{}, "weeks_of_year": []int{}, "months": []int{}, "positions": []int{}}}
			data["alarms"] = []any{map[string]any{"relative_offset": -900, "absolute_date": nil, "proximity": 0}, map[string]any{"relative_offset": 0, "absolute_date": "2028-01-02T14:00:00Z", "proximity": 0}, map[string]any{"relative_offset": 0, "absolute_date": nil, "proximity": 2, "location": map[string]any{"title": "Bridge Test park", "radius": 150, "latitude": 51.5, "longitude": -0.1}}}
		} else {
			data["recurrence"] = []any{map[string]any{"frequency": "constructor", "interval": 1}}
			data["alarms"] = []any{map[string]any{"proximity": 99, "location": map[string]any{"title": "Unproven location", "radius": 20}}}
		}
		records = append(records, rmObservation("reminders.reminder", fmt.Sprintf("web-rem-details-%d", index), data))
	}
	err := w.ControlAction("/dataset", simapple.Dataset{Records: records})
	return map[string]any{"ok": err == nil}, err
}
