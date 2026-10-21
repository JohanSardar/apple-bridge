package scenarios

import (
	"context"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func reminderReadModelGroups(ctx context.Context, w *world.World, listID string, add func(string, bool)) {
	for _, protocol := range []string{"rest", "mcp"} {
		client := scripted.Client{Endpoint: w.Gateway, Protocol: protocol}
		for _, test := range []struct {
			selector string
			value    string
			count    int
			kind     string
			sections int
		}{
			{"smart", "today", 2, "none", 1},
			{"smart", "scheduled", 3, "date", 3},
			{"smart", "all", 4, "list", 2},
			{"smart", "completed", 1, "list", 1},
			{"smart", "flagged", 1, "none", 1},
			{"smart", "assigned", 1, "none", 1},
			{"tag", "work", 2, "none", 1},
		} {
			var page struct {
				Count    int
				Sections []struct {
					Section struct {
						Kind   string
						ListID string `json:"list_id"`
					}
				}
			}
			err := client.Call(ctx, "reminders.items", map[string]any{"time_zone": "America/New_York", test.selector: test.value}, &page)
			valid := err == nil && page.Count == test.count && len(page.Sections) == test.sections
			for _, section := range page.Sections {
				valid = valid && section.Section.Kind == test.kind && (test.kind != "list" || section.Section.ListID != "")
			}
			add(protocol+"_"+test.value+"_grouping_and_count", valid)
		}
		var completed struct {
			Count int
		}
		err := client.Call(ctx, "reminders.items", map[string]any{"time_zone": "America/New_York", "list": listID, "show_completed": true}, &completed)
		add(protocol+"_show_completed_before_nesting", err == nil && completed.Count == 3)
		var priorities struct {
			Sections []struct {
				Section struct {
					ListID string `json:"list_id"`
				}
				Items []struct{ Reminder contracts.Record }
			}
		}
		err = client.Call(ctx, "reminders.items", map[string]any{"time_zone": "America/New_York", "smart": "all", "sort": "priority"}, &priorities)
		valid := err == nil
		found := false
		for _, section := range priorities.Sections {
			if len(section.Items) == 2 && recordTitle(section.Items[0].Reminder) == "Bridge Test Undated" {
				found = recordTitle(section.Items[1].Reminder) == "Bridge Test Future"
			}
		}
		add(protocol+"_apple_priority_none_sorts_after_low", valid && found)
	}
}
