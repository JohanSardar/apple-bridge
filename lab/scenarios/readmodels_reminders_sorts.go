package scenarios

import (
	"context"
	"encoding/json"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func reminderReadModelSorts(ctx context.Context, w *world.World, add func(string, bool)) {
	fixtures := reminderReadModelFixtures()
	add("sort_list_capture", w.ControlAction("/external-change", map[string]any{"collection": "reminders.list", "key": "rm-rem-sort-list", "data": map[string]any{"title": "Bridge Test Sorts", "sections": []any{}}}) == nil)
	add("sort_list_converged", w.Await(5*time.Second, func() bool { return convergence(w) }) == nil)
	listID := ""
	records, err := w.Records(w.Gateway)
	for _, record := range records {
		if record.Collection == "reminders.list" && recordTitle(record) == "Bridge Test Sorts" {
			listID = record.ID
		}
	}
	add("sort_list_reference", err == nil && listID != "")
	for index, fixture := range []contracts.Observation{fixtures[7], fixtures[8], fixtures[7], fixtures[7]} {
		var data map[string]any
		json.Unmarshal(fixture.Data, &data)
		data["list_id"] = rmRef("reminders.list", "rm-rem-sort-list")
		key := []string{"rm-rem-sort-future", "rm-rem-sort-undated", "rm-rem-sort-high", "rm-rem-sort-medium"}[index]
		switch index {
		case 0:
			data["manual_order"], data["created_at"] = 2, "2026-01-02T12:00:00Z"
		case 1:
			data["manual_order"], data["created_at"] = 1, nil
		case 2:
			data["title"], data["priority"], data["manual_order"], data["created_at"], data["due"] = "Bridge Test High", 1, 3, "2026-01-01T12:00:00Z", nil
		case 3:
			data["title"], data["priority"], data["manual_order"], data["created_at"], data["due"] = "Bridge Test Medium", 5, nil, "2026-01-03T12:00:00Z", nil
		}
		add("sort_fixture_capture", w.ControlAction("/external-change", map[string]any{"collection": "reminders.reminder", "key": key, "data": data}) == nil)
	}
	add("sort_fixtures_converged", w.Await(5*time.Second, func() bool { return convergence(w) }) == nil)
	for _, protocol := range []string{"rest", "mcp"} {
		client := scripted.Client{Endpoint: w.Gateway, Protocol: protocol}
		for _, test := range []struct {
			sort   string
			titles []string
		}{
			{"manual", []string{"Bridge Test Undated", "Bridge Test Future", "Bridge Test High", "Bridge Test Medium"}},
			{"created", []string{"Bridge Test High", "Bridge Test Future", "Bridge Test Medium", "Bridge Test Undated"}},
			{"priority", []string{"Bridge Test High", "Bridge Test Medium", "Bridge Test Undated", "Bridge Test Future"}},
			{"title", []string{"Bridge Test Future", "Bridge Test High", "Bridge Test Medium", "Bridge Test Undated"}},
			{"due", nil},
		} {
			var page struct {
				Count    int
				Sections []struct {
					Items []struct{ Reminder contracts.Record }
				}
			}
			err := client.Call(ctx, "reminders.items", map[string]any{"list": listID, "time_zone": "America/New_York", "sort": test.sort}, &page)
			valid := err == nil && page.Count == 4 && len(page.Sections) == 1 && len(page.Sections[0].Items) == 4
			if valid {
				for index, item := range page.Sections[0].Items {
					if test.sort == "due" {
						valid = valid && (index != 0 || recordTitle(item.Reminder) == "Bridge Test Future")
					} else {
						valid = valid && recordTitle(item.Reminder) == test.titles[index]
					}
				}
			}
			add(protocol+"_"+test.sort+"_sort_and_missing_values_last", valid)
		}
	}
}
