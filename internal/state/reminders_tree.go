package state

import (
	"cmp"
	"slices"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"golang.org/x/text/collate"
	"golang.org/x/text/language"
)

func reminderTree(nodes map[string]reminderNode, ids []string, attachments map[string][]contracts.Record, sort string, zone *time.Location) []contracts.ReminderItem {
	parents := map[string]string{}
	for _, id := range ids {
		node := nodes[id]
		if parent, exists := nodes[node.fields.ParentID]; exists && parent.group == node.group {
			parents[id] = parent.record.ID
		}
	}
	slices.Sort(ids)
	done := map[string]bool{}
	for _, id := range ids {
		path, positions := []string{}, map[string]int{}
		for next := id; next != "" && !done[next]; next = parents[next] {
			if position, found := positions[next]; found {
				delete(parents, slices.Min(path[position:]))
				break
			}
			positions[next] = len(path)
			path = append(path, next)
		}
		for _, visited := range path {
			done[visited] = true
		}
	}
	children := map[string][]string{}
	for _, id := range ids {
		children[parents[id]] = append(children[parents[id]], id)
	}
	orderItems := func(a, b reminderFields) int { return reminderItemOrder(a, b, sort, zone) }
	if sort == "title" {
		comparer := collate.New(language.Und, collate.IgnoreCase, collate.Numeric)
		orderItems = func(a, b reminderFields) int { return comparer.CompareString(a.Title, b.Title) }
	}
	for _, siblings := range children {
		slices.SortFunc(siblings, func(a, b string) int {
			if order := orderItems(nodes[a].fields, nodes[b].fields); order != 0 {
				return order
			}
			return cmp.Compare(a, b)
		})
	}
	var build func(string) []contracts.ReminderItem
	build = func(parent string) []contracts.ReminderItem {
		items := make([]contracts.ReminderItem, 0, len(children[parent]))
		for _, id := range children[parent] {
			media := attachments[id]
			if media == nil {
				media = []contracts.Record{}
			}
			items = append(items, contracts.ReminderItem{Reminder: nodes[id].record, Subtasks: build(id), Attachments: media})
		}
		return items
	}
	return build("")
}

func reminderItemOrder(a, b reminderFields, sort string, zone *time.Location) int {
	switch sort {
	case "priority":
		rank := func(priority int) int {
			if priority == 0 {
				return 10
			}
			return priority
		}
		return cmp.Compare(rank(a.Priority), rank(b.Priority))
	case "due", "created":
		left, right := a.Due, b.Due
		if sort == "created" {
			left, right = a.CreatedAt, b.CreatedAt
		}
		parse := func(value string) time.Time {
			if instant, err := time.Parse(time.RFC3339Nano, value); err == nil {
				return instant
			}
			instant, _ := time.ParseInLocation(time.DateOnly, value, zone)
			return instant
		}
		first, second := parse(left), parse(right)
		if first.IsZero() != second.IsZero() {
			if first.IsZero() {
				return 1
			}
			return -1
		}
		return first.Compare(second)
	default:
		if (a.Order == nil) != (b.Order == nil) {
			if a.Order == nil {
				return 1
			}
			return -1
		}
		if a.Order != nil {
			return cmp.Compare(*a.Order, *b.Order)
		}
		return 0
	}
}
