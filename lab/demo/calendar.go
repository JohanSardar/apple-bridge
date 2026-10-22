package demo

import (
	"fmt"
	"time"
)

func (d *Dataset) calendars() {
	calendars := []struct {
		key, title, color, kind string
		writable                bool
	}{
		{"demo-cal-work", "Work", "#f08a24", "caldav", true},
		{"demo-cal-personal", "Personal", "#4b8cf5", "caldav", true},
		{"demo-cal-family", "Family", "#b066d1", "caldav", true},
		{"demo-cal-birthdays", "Birthdays", "#e4af26", "birthday", false},
		{"demo-cal-holidays", "Holidays", "#5fa16e", "subscribed", false},
	}
	for _, calendar := range calendars {
		d.put("calendar.calendar", calendar.key, map[string]any{"title": calendar.title, "account": "demo-account", "color": calendar.color, "type": calendar.kind, "writable": calendar.writable})
	}
	midnight := time.Date(Epoch.Year(), Epoch.Month(), Epoch.Day(), 0, 0, 0, 0, time.UTC)
	for day := -14; day <= 42; day++ {
		date := midnight.AddDate(0, 0, day)
		for index := 0; index < 3; index++ {
			calendar := calendars[index]
			title := []string{"Project check-in", "Lunch with Sam", "Evening walk"}[index]
			start := date.Add(time.Duration([]int{14, 17, 22}[index]) * time.Hour)
			fields := map[string]any{"calendar_id": reference("calendar.calendar", calendar.key), "title": title, "start": start.Format(time.RFC3339), "end": start.Add(time.Hour).Format(time.RFC3339), "all_day": false, "time_zone": "America/New_York", "location": []string{"Studio", "Corner café", "Riverside park"}[index], "notes": "", "url": nil, "status": 1, "availability": 0, "is_exception": false, "attendees": []any{}, "alarms": []any{}, "recurrence": []any{}, "store_status": "partial", "store_coverage": map[string]any{"travel_time": map[string]any{"status": "unsupported", "reason": "positive native fixture pending"}, "structured_location": map[string]any{"status": "unsupported", "reason": "native coordinate units unverified"}}, "attachments": []any{}}
			if index == 0 {
				fields["recurrence"] = []any{map[string]any{"frequency": "daily", "interval": 1, "count": 0}}
				fields["series_id"] = "demo-daily-checkin"
				fields["occurrence_date"] = start.Format(time.RFC3339)
				fields["notes"] = "Review the draft, then choose the next small step."
				fields["alarms"] = []any{map[string]any{"relative_offset": -600}}
			}
			if index == 1 {
				fields["attendees"] = []any{map[string]any{"name": "Sam Example", "url": "mailto:sam@example.com", "status": 2, "role": 1, "type": 1}}
			}
			d.put("calendar.event", fmt.Sprintf("demo-cal-event-%d-%d", day, index), fields)
		}
		if day%7 == 0 {
			d.put("calendar.event", fmt.Sprintf("demo-cal-day-%d", day), map[string]any{"calendar_id": reference("calendar.calendar", "demo-cal-family"), "title": "A day together", "start": date.Format(time.DateOnly), "end": date.AddDate(0, 0, 1).Format(time.DateOnly), "all_day": true, "time_zone": nil, "status": 1, "availability": 0, "is_exception": false, "recurrence": []any{}, "alarms": []any{}, "attendees": []any{}})
		}
	}
	d.put("calendar.event", "demo-cal-trip", map[string]any{"calendar_id": reference("calendar.calendar", "demo-cal-personal"), "title": "Weekend away", "start": midnight.AddDate(0, 0, 4).Format(time.DateOnly), "end": midnight.AddDate(0, 0, 7).Format(time.DateOnly), "all_day": true, "time_zone": nil, "status": 1, "location": "By the sea", "notes": "Tickets and a good book.", "recurrence": []any{}, "alarms": []any{}, "attendees": []any{}})
	d.put("calendar.event", "demo-cal-overlap", map[string]any{"calendar_id": reference("calendar.calendar", "demo-cal-work"), "title": "Design review", "start": midnight.Add(14*time.Hour + 30*time.Minute).Format(time.RFC3339), "end": midnight.Add(16 * time.Hour).Format(time.RFC3339), "all_day": false, "time_zone": "America/New_York", "status": 2, "location": "Studio", "recurrence": []any{}, "alarms": []any{}, "attendees": []any{}})
}
