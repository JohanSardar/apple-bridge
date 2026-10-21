package scenarios

import (
	"context"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func reminderReadModelZones(ctx context.Context, w *world.World, add func(string, bool)) {
	for key, due := range map[string]string{"before": "2026-03-08T04:30:00Z", "after": "2026-03-08T07:30:00Z"} {
		data := map[string]any{"title": "Bridge Test DST " + key, "list_id": rmRef("reminders.list", "rm-rem-other"), "completed": false, "flagged": false, "due": due, "due_all_day": false, "assignment": nil, "store_coverage": map[string]any{"assignment": map[string]any{"status": "complete"}}}
		add("dst_fixture_"+key, w.ControlAction("/external-change", map[string]any{"collection": "reminders.reminder", "key": "rm-rem-dst-" + key, "data": data}) == nil)
	}
	add("dst_fixtures_converged", w.Await(5*time.Second, func() bool { return convergence(w) }) == nil)
	for _, protocol := range []string{"rest", "mcp"} {
		client := scripted.Client{Endpoint: w.Gateway, Protocol: protocol}
		for _, zone := range []string{"UTC", "America/New_York"} {
			var page struct {
				Sections []struct {
					Section struct{ ID, Kind string }
					Items   []struct{ Reminder contracts.Record }
				}
			}
			err := client.Call(ctx, "reminders.items", map[string]any{"smart": "scheduled", "time_zone": zone}, &page)
			groups := map[string]string{}
			for _, section := range page.Sections {
				if section.Section.Kind != "date" {
					continue
				}
				for _, item := range section.Items {
					groups[recordTitle(item.Reminder)] = section.Section.ID
				}
			}
			before, after := groups["Bridge Test DST before"], groups["Bridge Test DST after"]
			expected := before == after
			if zone == "America/New_York" {
				expected = before != after
			}
			add(protocol+"_viewer_zone_across_dst_"+zone, err == nil && before != "" && after != "" && expected)
		}
	}
}
