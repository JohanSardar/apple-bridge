package state

import (
	"encoding/json"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

type reminderFields struct {
	Title      string   `json:"title"`
	ParentID   string   `json:"parent_id"`
	SectionID  string   `json:"section_id"`
	Order      *float64 `json:"manual_order"`
	CreatedAt  string   `json:"created_at"`
	Priority   int      `json:"priority"`
	ListID     string   `json:"list_id"`
	Completed  bool     `json:"completed"`
	Flagged    *bool    `json:"flagged"`
	Due        string   `json:"due"`
	Tags       []string `json:"tags"`
	Assignment *struct {
		Assignee *struct {
			ID   string `json:"id"`
			IsMe *bool  `json:"is_me"`
		} `json:"assignee"`
	} `json:"assignment"`
	Coverage map[string]struct {
		Status string `json:"status"`
	} `json:"store_coverage"`
}

type reminderListFields struct {
	ParentID    *string                     `json:"parent_id"`
	Group       *bool                       `json:"is_group"`
	Order       *float64                    `json:"display_order"`
	Pinned      *bool                       `json:"pinned"`
	SmartType   json.RawMessage             `json:"smart_list_type"`
	SmartFilter json.RawMessage             `json:"smart_filter"`
	Sections    []contracts.ReminderSection `json:"sections"`
}

func reminderZone(name string) (*time.Location, error) {
	if name == "" || name == "Local" {
		return nil, contracts.ErrInvalid
	}
	zone, err := time.LoadLocation(name)
	if err != nil {
		return nil, contracts.ErrInvalid
	}
	return zone, nil
}

func reminderDueDay(due string, zone *time.Location) (string, bool) {
	if due == "" {
		return "", true
	}
	if date, err := time.Parse(time.DateOnly, due); err == nil {
		return date.Format(time.DateOnly), true
	}
	if date, err := time.Parse(time.RFC3339Nano, due); err == nil {
		return date.In(zone).Format(time.DateOnly), true
	}
	return "", false
}

func reminderAssignee(fields reminderFields) (bool, bool) {
	if fields.Assignment == nil {
		return false, fields.Coverage["assignment"].Status == "complete"
	}
	assignee := fields.Assignment.Assignee
	if assignee == nil || assignee.ID == "" || assignee.IsMe == nil {
		return false, false
	}
	return *assignee.IsMe, true
}

func reminderCustomList(fields reminderListFields) bool {
	present := func(raw json.RawMessage) bool { return len(raw) > 0 && string(raw) != "null" && string(raw) != `""` }
	return present(fields.SmartType) || present(fields.SmartFilter)
}
