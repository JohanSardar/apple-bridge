package scenarios

import (
	"context"
	"fmt"
	"reflect"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func checkRemindersDemoReadModels(ctx context.Context, w *world.World, add func(string, bool)) {
	endpoints := append([]world.Endpoint{w.Gateway}, w.Replicas...)
	var expectedLists *contracts.ReminderListsResult
	expectedItems := map[string]contracts.ReminderItemsResult{}
	for index, endpoint := range endpoints {
		for _, protocol := range []string{"rest", "mcp"} {
			prefix := fmt.Sprintf("demo_readmodels_%d_%s_", index, protocol)
			client := scripted.Client{Endpoint: endpoint, Protocol: protocol}
			var lists contracts.ReminderListsResult
			err := client.Call(ctx, "reminders.lists", contracts.ReminderListsQuery{TimeZone: "America/New_York"}, &lists)
			add(prefix+"lists_call", err == nil)
			if err != nil {
				continue
			}
			lists.Cursor = contracts.Cursor{}
			if expectedLists == nil {
				copy := lists
				expectedLists = &copy
			}
			add(prefix+"lists_parity", reflect.DeepEqual(*expectedLists, lists))
			titles := []string{}
			byTitle := map[string]contracts.ReminderList{}
			for _, list := range lists.Lists {
				title := recordTitle(list.List)
				titles = append(titles, title)
				byTitle[title] = list
			}
			add(prefix+"sidebar_order", reflect.DeepEqual(titles, []string{"Travel", "Home", "Personal", "Groceries", "Projects", "Flagged errands"}))
			home := byTitle["Home"]
			groups := home.Count == nil && rmData(home.List)["is_group"] == true
			orders := map[string]float64{"Home": 0, "Personal": 1, "Groceries": 2, "Projects": 3, "Travel": 4, "Flagged errands": 5}
			for title, list := range byTitle {
				groups = groups && list.Order != nil && *list.Order == orders[title] && list.Pinned != nil && *list.Pinned == (title == "Travel")
				if title == "Personal" || title == "Groceries" {
					groups = groups && list.GroupID != nil && *list.GroupID == home.List.ID
				} else {
					groups = groups && list.GroupID == nil
				}
			}
			add(prefix+"groups_order_and_pin", groups)
			for title, names := range map[string][]string{"Personal": {"This week", "Later"}, "Groceries": {"Produce", "Pantry"}} {
				sections := byTitle[title].Sections
				exact := len(sections) == len(names)
				for i, section := range sections {
					exact = exact && i < len(names) && section.Title == names[i] && section.Kind == "store" && section.Order == float64(i)
				}
				add(prefix+title+"_sections", exact)
			}
			custom := byTitle["Flagged errands"]
			add(prefix+"custom_unsupported", custom.Status == "unsupported" && custom.Count == nil)
			counts := map[string]int{"scheduled": 4, "flagged": 2, "all": 9, "completed": 1, "assigned": 1}
			found := map[string]int{}
			for _, tile := range lists.Smart {
				if _, ok := counts[tile.ID]; ok && tile.Status == "" && tile.Count != nil {
					found[tile.ID] = *tile.Count
				}
			}
			add(prefix+"smart_counts", reflect.DeepEqual(found, counts))
			tags := map[string]int{}
			for _, tag := range lists.Tags {
				tags[tag.Name] = tag.Count
			}
			add(prefix+"tags_incomplete_exact", len(lists.Tags) == 4 && reflect.DeepEqual(tags, map[string]int{"home": 2, "work": 1, "travel": 3, "errands": 2}))
			queries := map[string]contracts.ReminderItemsQuery{
				"travel": {List: byTitle["Travel"].List.ID}, "groceries": {List: byTitle["Groceries"].List.ID},
				"assigned": {Smart: "assigned"}, "personal_incomplete": {List: byTitle["Personal"].List.ID},
				"personal_completed": {List: byTitle["Personal"].List.ID, ShowCompleted: true},
			}
			for name, query := range queries {
				query.TimeZone = "America/New_York"
				var items contracts.ReminderItemsResult
				err := client.Call(ctx, "reminders.items", query, &items)
				add(prefix+name+"_call", err == nil)
				if err != nil {
					continue
				}
				items.Cursor = contracts.Cursor{}
				if _, ok := expectedItems[name]; !ok {
					expectedItems[name] = items
				}
				add(prefix+name+"_parity", reflect.DeepEqual(expectedItems[name], items))
				add(prefix+name+"_exact", demoReminderItemsExact(name, items))
			}
		}
	}
}

func demoReminderItemsExact(name string, result contracts.ReminderItemsResult) bool {
	titles := [][]string{}
	for _, section := range result.Sections {
		row := []string{}
		for _, item := range section.Items {
			row = append(row, recordTitle(item.Reminder))
		}
		titles = append(titles, row)
	}
	switch name {
	case "travel":
		if result.Count != 3 || len(result.Sections) != 1 || len(result.Sections[0].Items) != 1 {
			return false
		}
		items := result.Sections[0].Items
		for _, title := range []string{"Plan a weekend trip", "Book a place to stay", "Compare two quiet cabins"} {
			if len(items) != 1 || recordTitle(items[0].Reminder) != title {
				return false
			}
			items = items[0].Subtasks
		}
		return len(items) == 0
	case "groceries":
		return result.Count == 3 && len(result.Sections) == 2 && result.Sections[0].Section.Kind == "store" && result.Sections[0].Section.Title == "Produce" && result.Sections[1].Section.Kind == "store" && result.Sections[1].Section.Title == "Pantry" && reflect.DeepEqual(titles, [][]string{{"Pick up dinner ingredients", "Tomatoes and basil"}, {"Pasta and olive oil"}})
	case "assigned":
		return result.Count == 1 && reflect.DeepEqual(titles, [][]string{{"Plan a weekend trip"}}) && len(result.Sections[0].Items[0].Subtasks) == 0
	case "personal_incomplete":
		return result.Count == 2 && reflect.DeepEqual(titles, [][]string{{"Take a long walk"}, {"Water the plants"}})
	case "personal_completed":
		return result.Count == 3 && reflect.DeepEqual(titles, [][]string{{"Take a long walk", "Choose the next book"}, {"Water the plants"}})
	}
	return false
}
