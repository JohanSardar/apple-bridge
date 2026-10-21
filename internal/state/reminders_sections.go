package state

import (
	"cmp"
	"context"
	"database/sql"
	"encoding/json"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func reminderItemSection(ctx context.Context, tx *sql.Tx, fields reminderFields, query contracts.ReminderItemsQuery, zone *time.Location, sections map[string]contracts.ReminderSection) (contracts.ReminderSection, error) {
	if query.List != "" {
		if section, exists := sections[fields.SectionID]; exists && section.Kind == "store" {
			return section, nil
		}
	}
	if query.Smart == "scheduled" {
		day, _ := reminderDueDay(fields.Due, zone)
		return contracts.ReminderSection{Kind: "date", ID: day, Title: day}, nil
	}
	if query.Smart == "all" || query.Smart == "completed" {
		if section, exists := sections[fields.ListID]; exists {
			return section, nil
		}
		list, err := modelRecord(ctx, tx, "reminders.list", fields.ListID)
		if err != nil {
			return contracts.ReminderSection{}, err
		}
		var data struct {
			Title string  `json:"title"`
			Order float64 `json:"display_order"`
		}
		if err = json.Unmarshal(list.Data, &data); err != nil {
			return contracts.ReminderSection{}, err
		}
		return contracts.ReminderSection{Kind: "list", ID: fields.ListID, Title: data.Title, Order: data.Order, ListID: fields.ListID}, nil
	}
	return contracts.ReminderSection{Kind: "none", ID: "none"}, nil
}

func reminderSectionOrder(a, b contracts.ReminderSection) int {
	if (a.Kind == "none") != (b.Kind == "none") {
		if a.Kind == "none" {
			return 1
		}
		return -1
	}
	if order := cmp.Compare(a.Order, b.Order); order != 0 {
		return order
	}
	return cmp.Compare(a.ID, b.ID)
}

func reminderItemAttachments(ctx context.Context, tx *sql.Tx, nodes map[string]reminderNode) (map[string][]contracts.Record, error) {
	result := map[string][]contracts.Record{}
	if len(nodes) == 0 {
		return result, nil
	}
	ids := make([]string, 0, len(nodes))
	for id := range nodes {
		ids = append(ids, id)
	}
	encoded, err := json.Marshal(ids)
	if err != nil {
		return nil, err
	}
	records, err := readModelRecords(ctx, tx, "SELECT payload FROM records WHERE collection='reminders.attachment' AND deleted=0 AND json_extract(payload,'$.data.reminder_id') IN (SELECT value FROM json_each(?)) ORDER BY id", string(encoded))
	if err != nil {
		return nil, err
	}
	for _, record := range records {
		var fields struct {
			ReminderID string `json:"reminder_id"`
		}
		if err = json.Unmarshal(record.Data, &fields); err != nil {
			return nil, err
		}
		if _, exists := nodes[fields.ReminderID]; exists {
			result[fields.ReminderID] = append(result[fields.ReminderID], record)
		}
	}
	return result, nil
}
