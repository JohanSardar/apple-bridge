package contracts

import (
	"encoding/json"
	"strings"
	"time"
)

type CalendarArgs struct {
	ID           string                `json:"id,omitempty"`
	CalendarID   string                `json:"calendar_id,omitempty"`
	Span         string                `json:"span,omitempty"`
	Title        *string               `json:"title,omitempty"`
	Notes        *string               `json:"notes,omitempty"`
	Location     *string               `json:"location,omitempty"`
	URL          *string               `json:"url,omitempty"`
	Start        *string               `json:"start,omitempty"`
	End          *string               `json:"end,omitempty"`
	AllDay       *bool                 `json:"all_day,omitempty"`
	TimeZone     *string               `json:"time_zone,omitempty"`
	Recurrence   *[]CalendarRecurrence `json:"recurrence,omitempty"`
	Alarms       *[]CalendarAlarm      `json:"alarms,omitempty"`
	Availability *int                  `json:"availability,omitempty"`
}
type CalendarRecurrence struct {
	Frequency string `json:"frequency"`
	Interval  int    `json:"interval"`
	Count     int    `json:"count,omitempty"`
	Until     string `json:"end,omitempty"`
}
type CalendarAlarm struct {
	RelativeOffset *float64 `json:"relative_offset,omitempty"`
	AbsoluteDate   string   `json:"absolute_date,omitempty"`
}
type AgendaQuery struct {
	Start      string `json:"start"`
	End        string `json:"end"`
	CalendarID string `json:"calendar_id,omitempty"`
}

func CalendarArgumentFields(op string) []string {
	switch op {
	case "calendar.create_calendar":
		return []string{"title"}
	case "calendar.delete_event":
		return []string{"id", "span"}
	case "calendar.create_event":
		return []string{"calendar_id", "title", "notes", "location", "url", "start", "end", "all_day", "time_zone", "recurrence", "alarms", "availability"}
	case "calendar.update_event":
		return []string{"id", "span", "calendar_id", "title", "notes", "location", "url", "start", "end", "all_day", "time_zone", "recurrence", "alarms", "availability"}
	}
	return nil
}
func ValidateCalendar(c Command) (CalendarArgs, error) {
	var a CalendarArgs
	if DecodeInput(c.Args, &a) != nil {
		return a, ErrInvalid
	}
	if a.Span == "all" {
		return a, ErrUnsupported
	}
	allowed := map[string]bool{}
	for _, k := range CalendarArgumentFields(c.Op) {
		allowed[k] = true
	}
	var fields map[string]json.RawMessage
	json.Unmarshal(c.Args, &fields)
	for k, v := range fields {
		if !allowed[k] || string(v) == "null" {
			return a, ErrInvalid
		}
	}
	if a.Title != nil && strings.TrimSpace(*a.Title) == "" {
		return a, ErrInvalid
	}
	switch c.Op {
	case "calendar.create_calendar":
		if a.Title == nil || c.ExpectedVersion != 0 {
			return a, ErrInvalid
		}
	case "calendar.create_event":
		if a.CalendarID == "" || a.Title == nil || a.Start == nil || a.End == nil || c.ExpectedVersion != 0 {
			return a, ErrInvalid
		}
	case "calendar.update_event", "calendar.delete_event":
		if a.ID == "" || c.ExpectedVersion == 0 || (a.Span != "this" && a.Span != "future") {
			return a, ErrInvalid
		}
		if c.Op == "calendar.update_event" && len(fields) < 3 {
			return a, ErrInvalid
		}
	default:
		return a, ErrUnsupported
	}
	allDay := a.AllDay != nil && *a.AllDay
	layout := time.RFC3339Nano
	if allDay {
		layout = time.DateOnly
	}
	var start, end time.Time
	for k, v := range map[string]*string{"start": a.Start, "end": a.End} {
		if v == nil {
			continue
		}
		d, e := time.Parse(layout, *v)
		if e != nil && a.AllDay == nil && c.Op == "calendar.update_event" {
			d, e = time.Parse(time.DateOnly, *v)
		}
		if e != nil {
			return a, ErrInvalid
		}
		if k == "start" {
			start = d
		} else {
			end = d
		}
	}
	if !start.IsZero() && !end.IsZero() && !end.After(start) {
		return a, ErrInvalid
	}
	if a.TimeZone != nil && *a.TimeZone != "" {
		if _, err := time.LoadLocation(*a.TimeZone); err != nil {
			return a, ErrInvalid
		}
	}
	if a.Recurrence != nil {
		for _, rule := range *a.Recurrence {
			if rule.Interval < 1 || rule.Count < 0 || (rule.Frequency != "daily" && rule.Frequency != "weekly" && rule.Frequency != "monthly" && rule.Frequency != "yearly") {
				return a, ErrInvalid
			}
			if rule.Until != "" {
				if _, e := time.Parse(time.RFC3339Nano, rule.Until); e != nil {
					return a, ErrInvalid
				}
			}
		}
	}
	if a.Alarms != nil {
		for _, alarm := range *a.Alarms {
			if (alarm.AbsoluteDate == "") == (alarm.RelativeOffset == nil) {
				return a, ErrInvalid
			}
			if alarm.AbsoluteDate != "" {
				if _, err := time.Parse(time.RFC3339Nano, alarm.AbsoluteDate); err != nil {
					return a, ErrInvalid
				}
			}
		}
	}
	if a.Availability != nil && (*a.Availability < -1 || *a.Availability > 3) {
		return a, ErrInvalid
	}
	return a, nil
}
func CalendarDescriptors() []OperationDescriptor {
	result := []OperationDescriptor{{Name: "calendar.agenda", Version: 1, Kind: ReadOp, Source: Calendar, Scope: "calendar:read", Input: AgendaQuery{}, Output: ReadResult{}, Description: "Read local occurrences overlapping a time range; all-day dates remain floating"}}
	for _, op := range []string{"create_event", "update_event", "delete_event", "create_calendar"} {
		name := "calendar." + op
		result = append(result, OperationDescriptor{Name: name, Version: 1, Kind: CommandOp, Source: Calendar, Scope: "calendar:write", Capability: name, Input: Command{}, Output: CommandRecord{}, Description: "Calendar " + op + "; update and delete require expected_version and span this or future"})
	}
	return result
}
