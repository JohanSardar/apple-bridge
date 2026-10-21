package scenarios

import (
	"context"
	"encoding/json"
	"reflect"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func reminderReadModelNesting(ctx context.Context, w *world.World, listID string, add func(string, bool)) {
	parents := map[string]string{"grandchild": "rm-rem-child", "cycle-a": "rm-rem-cycle-b", "cycle-b": "rm-rem-cycle-a"}
	dataFor := func(key string) map[string]any {
		return map[string]any{"title": "Bridge Test " + key, "list_id": rmRef("reminders.list", "list"), "completed": false, "section_id": "section-a"}
	}
	for key := range parents {
		add("nested_fixture_"+key, w.ControlAction("/external-change", map[string]any{"collection": "reminders.reminder", "key": "rm-rem-" + key, "data": dataFor(key)}) == nil)
	}
	add("nested_fixture_aliases_converged", w.Await(5*time.Second, func() bool { return convergence(w) }) == nil)
	for key, parent := range parents {
		data := dataFor(key)
		data["parent_id"] = rmRef("reminders.reminder", parent)
		add("nested_fixture_link_"+key, w.ControlAction("/external-change", map[string]any{"collection": "reminders.reminder", "key": "rm-rem-" + key, "data": data}) == nil)
	}
	add("nested_fixtures_converged", w.Await(5*time.Second, func() bool { return convergence(w) }) == nil)
	type item struct {
		Reminder contracts.Record
		Subtasks []item
	}
	type page struct {
		Count    int
		Sections []struct{ Items []item }
	}
	query := map[string]any{"time_zone": "America/New_York", "list": listID}
	for _, protocol := range []string{"rest", "mcp"} {
		client := scripted.Client{Endpoint: w.Gateway, Protocol: protocol}
		var first, second page
		err := client.Call(ctx, "reminders.items", query, &first)
		err2 := client.Call(ctx, "reminders.items", query, &second)
		seen := map[string]bool{}
		duplicate, deep := false, false
		var walk func([]item, int)
		walk = func(items []item, depth int) {
			for _, entry := range items {
				duplicate = duplicate || seen[entry.Reminder.ID]
				seen[entry.Reminder.ID] = true
				deep = deep || depth == 3 && recordTitle(entry.Reminder) == "Bridge Test grandchild"
				walk(entry.Subtasks, depth+1)
			}
		}
		for _, section := range first.Sections {
			walk(section.Items, 1)
		}
		add(protocol+"_recursive_subtasks_and_deterministic_cycle_break", err == nil && err2 == nil && first.Count == 5 && len(seen) == 5 && !duplicate && deep && reflect.DeepEqual(first, second))
	}
	fixture := reminderReadModelFixtures()[4]
	var data map[string]any
	json.Unmarshal(fixture.Data, &data)
	data["completed"] = true
	add("completed_parent_capture", w.ControlAction("/external-change", map[string]any{"collection": fixture.Collection, "key": fixture.NativeKey, "data": data}) == nil)
	add("completed_parent_converged", w.Await(5*time.Second, func() bool { return convergence(w) }) == nil)
	var filtered page
	err := (scripted.Client{Endpoint: w.Gateway}).Call(ctx, "reminders.items", query, &filtered)
	orphanRoot := false
	for _, section := range filtered.Sections {
		for _, root := range section.Items {
			orphanRoot = orphanRoot || recordTitle(root.Reminder) == "Bridge Test Child" && len(root.Subtasks) == 1
		}
	}
	add("completion_filter_before_recursive_nesting", err == nil && filtered.Count == 4 && orphanRoot)
}
