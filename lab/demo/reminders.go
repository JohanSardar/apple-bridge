package demo

import "time"

func reminderCoverage(fields ...string) map[string]any {
	coverage := map[string]any{}
	for _, field := range fields {
		coverage[field] = map[string]any{"status": "complete"}
	}
	return coverage
}

func (d *Dataset) reminders() {
	section := func(id, title string, order int) any {
		return map[string]any{"kind": "store", "id": id, "title": title, "order": order}
	}
	for index, list := range []struct {
		key, title, emblem string
		group, pinned      bool
		parent             string
		sections           []any
	}{
		{"demo-rem-home", "Home", "🏠", true, false, "", []any{}},
		{"list", "Personal", "☀️", false, false, "demo-rem-home", []any{section("demo-rem-now", "This week", 0), section("demo-rem-later", "Later", 1)}},
		{"demo-rem-groceries", "Groceries", "🛒", false, false, "demo-rem-home", []any{section("demo-rem-produce", "Produce", 0), section("demo-rem-pantry", "Pantry", 1)}},
		{"demo-rem-projects", "Projects", "💻", false, false, "", []any{}},
		{"demo-rem-travel", "Travel", "✈️", false, true, "", []any{}},
		{"demo-rem-smart", "Flagged errands", "🚩", false, false, "", []any{}},
	} {
		var parent any
		if list.parent != "" {
			parent = reference("reminders.list", list.parent)
		}
		fields := map[string]any{"title": list.title, "writable": !list.group, "color": "#4B8CF5", "account": "demo-account", "store_identifier": list.key, "is_group": list.group, "parent_id": parent, "badge_emblem": list.emblem, "display_order": index, "pinned": list.pinned, "pinned_at": nil, "sorting_style": "manual", "categorize_groceries": list.key == "demo-rem-groceries", "grocery_locale": "en_US", "sharing_status": 0, "shared_owner_name": nil, "smart_list_type": nil, "smart_filter": nil, "sections": list.sections, "store_color": nil, "store_status": "partial"}
		coverage := reminderCoverage("store_identifier", "is_group", "parent_id", "badge_emblem", "display_order", "pinned", "pinned_at", "sorting_style", "categorize_groceries", "grocery_locale", "sharing_status", "shared_owner_name", "smart_list_type", "sections")
		coverage["store_color"], coverage["smart_filter"] = map[string]any{"status": "unsupported"}, map[string]any{"status": "unsupported"}
		if list.pinned {
			fields["pinned_at"] = Epoch.Add(-24 * time.Hour)
		}
		if list.key == "demo-rem-smart" {
			fields["smart_list_type"] = "custom"
			fields["writable"] = false
		}
		fields["store_coverage"] = coverage
		d.put("reminders.list", list.key, fields)
	}
	assignment := func(id string, me bool) any {
		return map[string]any{"assignee": map[string]any{"id": id, "is_me": me}, "originator": nil, "assigned_at": Epoch}
	}
	for index, reminder := range []struct {
		key, title, list string
		extra            map[string]any
	}{
		{"demo-rem-plan", "Plan a weekend trip", "demo-rem-travel", map[string]any{"flagged": true, "due": Epoch.AddDate(0, 0, 3).Format(time.DateOnly), "tags": []string{"travel"}, "priority": 1, "assignment": assignment("demo-me", true)}},
		{"demo-rem-stay", "Book a place to stay", "demo-rem-travel", map[string]any{"parent_id": reference("reminders.reminder", "demo-rem-plan"), "tags": []string{"travel"}}},
		{"demo-rem-options", "Compare two quiet cabins", "demo-rem-travel", map[string]any{"parent_id": reference("reminders.reminder", "demo-rem-stay"), "tags": []string{"travel"}}},
		{"demo-rem-walk", "Take a long walk", "list", map[string]any{"section_id": "demo-rem-now", "due": Epoch.Format(time.DateOnly), "tags": []string{"home"}}},
		{"demo-rem-plants", "Water the plants", "list", map[string]any{"section_id": "demo-rem-later", "priority": 9, "tags": []string{"home"}}},
		{"demo-rem-market", "Pick up dinner ingredients", "demo-rem-groceries", map[string]any{"flagged": true, "section_id": "demo-rem-produce", "priority": 5, "tags": []string{"errands"}, "due": Epoch.AddDate(0, 0, 1).Format(time.DateOnly)}},
		{"demo-rem-tomatoes", "Tomatoes and basil", "demo-rem-groceries", map[string]any{"section_id": "demo-rem-produce", "tags": []string{"errands"}}},
		{"demo-rem-pasta", "Pasta and olive oil", "demo-rem-groceries", map[string]any{"section_id": "demo-rem-pantry"}},
		{"demo-rem-review", "Review the first draft", "demo-rem-projects", map[string]any{"due": Epoch.Add(48 * time.Hour), "due_all_day": false, "early_reminders": []int{-900}, "tags": []string{"work"}, "assignment": assignment("demo-other", false)}},
		{"demo-rem-done", "Choose the next book", "list", map[string]any{"completed": true, "completed_at": Epoch.Add(-time.Hour), "section_id": "demo-rem-now", "tags": []string{"home"}}},
	} {
		fields := map[string]any{"title": reminder.title, "list_id": reference("reminders.list", reminder.list), "completed": false, "completed_at": nil, "priority": 0, "due": nil, "due_all_day": true, "notes": nil, "url": nil, "flagged": false, "parent_id": nil, "section_id": nil, "tags": []string{}, "manual_order": index, "created_at": Epoch.Add(-48 * time.Hour), "early_reminders": []int{}, "assignment": nil, "store_alarms": []any{}, "store_recurrence": []any{}, "attachment_ids": []string{}, "store_identifier": reminder.key, "store_status": "complete"}
		for field, value := range reminder.extra {
			fields[field] = value
		}
		fields["store_coverage"] = reminderCoverage("store_identifier", "flagged", "parent_id", "section_id", "tags", "manual_order", "created_at", "early_reminders", "assignment", "store_alarms", "store_recurrence", "attachment_ids")
		d.put("reminders.reminder", reminder.key, fields)
	}
}
