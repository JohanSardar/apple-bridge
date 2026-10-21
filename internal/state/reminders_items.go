package state

import (
	"context"
	"database/sql"
	"encoding/json"
	"slices"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

type reminderNode struct {
	record contracts.Record
	fields reminderFields
	group  string
}

func (s *Store) ReminderItems(ctx context.Context, visibility trust.Visibility, query contracts.ReminderItemsQuery) (contracts.ReminderItemsResult, error) {
	result := contracts.ReminderItemsResult{Sections: []contracts.ReminderSectionItems{}}
	zone, err := reminderZone(query.TimeZone)
	selectors := 0
	for _, selector := range []string{query.List, query.Smart, query.Tag} {
		if selector != "" {
			selectors++
		}
	}
	if err != nil || selectors != 1 || !slices.Contains([]string{"", "manual", "due", "created", "priority", "title"}, query.Sort) || !slices.Contains([]string{"", "today", "scheduled", "all", "flagged", "completed", "assigned"}, query.Smart) {
		return result, contracts.ErrInvalid
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
	sections := map[string]contracts.ReminderSection{}
	if query.List != "" {
		list, err := modelRecord(ctx, tx, "reminders.list", query.List)
		if err != nil {
			return result, err
		}
		var fields reminderListFields
		if err = json.Unmarshal(list.Data, &fields); err != nil {
			return result, err
		}
		if reminderCustomList(fields) {
			return result, contracts.ErrUnsupported
		}
		for _, section := range fields.Sections {
			if section.Kind == "store" && section.ID != "" {
				sections[section.ID] = section
			}
		}
	}
	where, args := "", []any{}
	if query.List != "" {
		where, args = " AND json_extract(payload,'$.data.list_id')=?", []any{query.List}
	}
	records, err := readModelRecords(ctx, tx, "SELECT payload FROM records WHERE collection='reminders.reminder' AND deleted=0"+where+" ORDER BY id", args...)
	if err != nil {
		return result, err
	}
	identityVerified := false
	today := time.Now().In(zone).Format(time.DateOnly)
	nodes := map[string]reminderNode{}
	for _, record := range records {
		var fields reminderFields
		if err = json.Unmarshal(record.Data, &fields); err != nil {
			return result, err
		}
		if fields.Assignment != nil {
			_, known := reminderAssignee(fields)
			identityVerified = identityVerified || known
		}
		selected, known := reminderSelected(fields, query, zone, today)
		if !known {
			return result, contracts.ErrUnsupported
		}
		if !selected {
			continue
		}
		section, err := reminderItemSection(ctx, tx, fields, query, zone, sections)
		if err != nil {
			return result, err
		}
		sections[section.ID] = section
		nodes[record.ID] = reminderNode{record: record, fields: fields, group: section.ID}
	}
	if query.Smart == "assigned" && !identityVerified {
		return result, contracts.ErrUnsupported
	}
	attachments, err := reminderItemAttachments(ctx, tx, nodes)
	if err != nil {
		return result, err
	}
	result.Count = len(nodes)
	groups := map[string][]string{}
	for id, node := range nodes {
		groups[node.group] = append(groups[node.group], id)
	}
	for _, section := range sections {
		result.Sections = append(result.Sections, contracts.ReminderSectionItems{Section: section, Items: reminderTree(nodes, groups[section.ID], attachments, query.Sort, zone)})
	}
	slices.SortFunc(result.Sections, func(a, b contracts.ReminderSectionItems) int { return reminderSectionOrder(a.Section, b.Section) })
	return result, nil
}

func reminderSelected(fields reminderFields, query contracts.ReminderItemsQuery, zone *time.Location, today string) (bool, bool) {
	if query.Smart == "completed" {
		return fields.Completed, true
	}
	if fields.Completed && !query.ShowCompleted {
		return false, true
	}
	if query.Tag != "" {
		return slices.Contains(fields.Tags, query.Tag), fields.Tags != nil
	}
	switch query.Smart {
	case "today", "scheduled":
		day, known := reminderDueDay(fields.Due, zone)
		return day != "" && (query.Smart == "scheduled" || day <= today), known
	case "flagged":
		return fields.Flagged != nil && *fields.Flagged, fields.Flagged != nil
	case "assigned":
		return reminderAssignee(fields)
	default:
		return true, true
	}
}
