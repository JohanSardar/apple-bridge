package scenarios

import (
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func reminderReadModelFixtures() []contracts.Observation {
	zone, _ := time.LoadLocation("America/New_York")
	today := time.Now().In(zone)
	reminder := func(key, title, list string, extra map[string]any) contracts.Observation {
		data := map[string]any{"title": title, "list_id": rmRef("reminders.list", list), "completed": false, "flagged": false, "assignment": nil, "priority": 0, "tags": []string{}, "manual_order": 0, "created_at": "2026-01-01T12:00:00Z", "store_status": "partial", "store_coverage": map[string]any{"assignment": map[string]any{"status": "complete"}, "flagged": map[string]any{"status": "complete"}}}
		for field, value := range extra {
			data[field] = value
		}
		return rmObservation("reminders.reminder", key, data)
	}
	assignment := func(me bool) any {
		return map[string]any{"assignee": map[string]any{"id": map[bool]string{true: "fixture-self", false: "fixture-other"}[me], "is_me": me}, "originator": nil, "assigned_at": "2026-01-01T12:00:00Z"}
	}
	return []contracts.Observation{
		rmObservation("reminders.list", "rm-rem-group", map[string]any{"title": "Bridge Test Group", "is_group": true, "display_order": 10, "pinned": false}),
		rmObservation("reminders.list", "list", map[string]any{"title": "Bridge Test Read Models", "parent_id": rmRef("reminders.list", "rm-rem-group"), "display_order": 20, "pinned": false, "sections": []any{map[string]any{"kind": "store", "id": "section-a", "title": "Bridge Test Section", "order": 0}, map[string]any{"kind": "store", "id": "section-empty", "title": "Bridge Test Empty", "order": 1}}}),
		rmObservation("reminders.list", "rm-rem-other", map[string]any{"title": "Bridge Test Other", "display_order": 30, "pinned": false, "sections": []any{}}),
		rmObservation("reminders.list", "rm-rem-pinned", map[string]any{"title": "Bridge Test Pinned", "display_order": 100, "pinned": true, "pinned_at": "2026-01-01T12:00:00Z", "sections": []any{}}),
		reminder("rm-rem-parent", "Bridge Test Parent", "list", map[string]any{"flagged": true, "due": "2000-01-01", "due_all_day": true, "priority": 5, "section_id": "section-a", "tags": []string{"work"}, "assignment": assignment(true)}),
		reminder("rm-rem-child", "Bridge Test Child", "list", map[string]any{"parent_id": rmRef("reminders.reminder", "rm-rem-parent"), "due": today.Format("2006-01-02"), "due_all_day": true, "priority": 1, "section_id": "section-a", "tags": []string{"work"}, "assignment": assignment(false)}),
		reminder("rm-rem-completed", "Bridge Test Completed", "list", map[string]any{"completed": true, "completed_at": "2026-01-01T12:00:00Z", "flagged": true, "priority": 9, "assignment": assignment(true), "tags": []string{"home"}}),
		reminder("rm-rem-future", "Bridge Test Future", "rm-rem-other", map[string]any{"due": today.AddDate(0, 0, 1).Format("2006-01-02"), "due_all_day": true, "tags": []string{"errands"}}),
		reminder("rm-rem-undated", "Bridge Test Undated", "rm-rem-other", map[string]any{"priority": 9, "assignment": assignment(false), "tags": []string{"home"}}),
	}
}
