package state

import (
	"cmp"
	"context"
	"database/sql"
	"encoding/json"
	"slices"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

func (s *Store) ReminderLists(ctx context.Context, visibility trust.Visibility, query contracts.ReminderListsQuery) (contracts.ReminderListsResult, error) {
	result := contracts.ReminderListsResult{Smart: []contracts.ReminderSmartTile{}, Lists: []contracts.ReminderList{}, Tags: []contracts.ReminderTagCount{}}
	zone, err := reminderZone(query.TimeZone)
	if err != nil {
		return result, err
	}
	tx, err := s.db.BeginTx(ctx, &sql.TxOptions{ReadOnly: true})
	if err != nil {
		return result, err
	}
	defer tx.Rollback()
	result.Cursor, err = cursorAt(ctx, tx)
	if err != nil || !visibility[contracts.Reminders] {
		return result, err
	}
	reminders, err := readModelRecords(ctx, tx, "SELECT payload FROM records WHERE collection='reminders.reminder' AND deleted=0 ORDER BY id")
	if err != nil {
		return result, err
	}
	counts := newReminderCounts()
	today := time.Now().In(zone).Format(time.DateOnly)
	for _, reminder := range reminders {
		var fields reminderFields
		if err = json.Unmarshal(reminder.Data, &fields); err != nil {
			return result, err
		}
		counts.add(fields, zone, today)
	}
	result.Smart = counts.smart()
	lists, err := readModelRecords(ctx, tx, "SELECT payload FROM records WHERE collection='reminders.list' AND deleted=0 ORDER BY id")
	if err != nil {
		return result, err
	}
	for _, list := range lists {
		var fields reminderListFields
		if err = json.Unmarshal(list.Data, &fields); err != nil {
			return result, err
		}
		sections := fields.Sections
		if sections == nil {
			sections = []contracts.ReminderSection{}
		}
		slices.SortStableFunc(sections, func(a, b contracts.ReminderSection) int {
			if c := cmp.Compare(a.Order, b.Order); c != 0 {
				return c
			}
			return cmp.Compare(a.ID, b.ID)
		})
		entry := contracts.ReminderList{List: list, GroupID: fields.ParentID, Order: fields.Order, Pinned: fields.Pinned, Sections: sections}
		if reminderCustomList(fields) {
			entry.Status = "unsupported"
		} else if fields.Group == nil || !*fields.Group {
			count := counts.lists[list.ID]
			entry.Count = &count
		}
		result.Lists = append(result.Lists, entry)
	}
	slices.SortFunc(result.Lists, reminderSidebarOrder)
	if !counts.tagsUnsupported {
		for name, count := range counts.tags {
			result.Tags = append(result.Tags, contracts.ReminderTagCount{Name: name, Count: count})
		}
	}
	slices.SortFunc(result.Tags, func(a, b contracts.ReminderTagCount) int { return cmp.Compare(a.Name, b.Name) })
	return result, nil
}

func reminderSidebarOrder(a, b contracts.ReminderList) int {
	pinnedA, pinnedB := a.Pinned != nil && *a.Pinned, b.Pinned != nil && *b.Pinned
	if pinnedA != pinnedB {
		if pinnedA {
			return -1
		}
		return 1
	}
	if (a.Order == nil) != (b.Order == nil) {
		if a.Order != nil {
			return -1
		}
		return 1
	}
	if a.Order != nil && b.Order != nil {
		if order := cmp.Compare(*a.Order, *b.Order); order != 0 {
			return order
		}
	}
	return cmp.Compare(a.List.ID, b.List.ID)
}
