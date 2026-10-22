package state

import (
	"context"
	"database/sql"
	"encoding/json"
	"fmt"
	"slices"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

type calendarRangeNode struct {
	record     contracts.Record
	start, end time.Time
	allDay     bool
	column     int
}

func (s *Store) CalendarRange(ctx context.Context, visibility trust.Visibility, query contracts.CalendarRangeQuery) (contracts.CalendarRangeResult, error) {
	result := contracts.CalendarRangeResult{Events: []contracts.CalendarRangeEvent{}, Calendars: []contracts.Record{}, CoverageNotes: []string{}}
	start, a := time.Parse(time.RFC3339Nano, query.Start)
	end, b := time.Parse(time.RFC3339Nano, query.End)
	zone := time.UTC
	var zoneErr error
	if query.TimeZone != "" {
		zone, zoneErr = time.LoadLocation(query.TimeZone)
	}
	if a != nil || b != nil || zoneErr != nil || !end.After(start) || end.Sub(start) > 1464*24*time.Hour || len(query.Calendars) > 256 || len(query.Text) > 256 {
		return result, contracts.ErrInvalid
	}
	tx, err := s.db.BeginTx(ctx, &sql.TxOptions{ReadOnly: true})
	if err != nil {
		return result, err
	}
	defer tx.Rollback()
	result.Cursor, err = cursorAt(ctx, tx)
	if err != nil || !visibility[contracts.Calendar] {
		return result, err
	}
	var revision uint64
	if err = tx.QueryRowContext(ctx, "SELECT coalesce(max(seq),0) FROM records WHERE collection IN ('calendar.calendar','calendar.event')").Scan(&revision); err != nil {
		return result, err
	}
	query.Calendars = slices.Clone(query.Calendars)
	slices.Sort(query.Calendars)
	query.Calendars = slices.Compact(query.Calendars)
	keyBytes, _ := json.Marshal(query)
	cacheKey := string(keyBytes)
	cacheRevision := fmt.Sprintf("%s/%d/%d", result.Cursor.Epoch, result.Cursor.SubscriptionRevision, revision)
	if cached, ok := s.calendarCache.get(cacheRevision, cacheKey); ok {
		cached.Cursor = result.Cursor
		return cached, nil
	}
	result.Calendars, err = readModelRecords(ctx, tx, "SELECT payload FROM records WHERE collection='calendar.calendar' AND deleted=0 ORDER BY id")
	if err != nil {
		return result, err
	}
	civilStart := start.In(zone).Format(time.DateOnly)
	localEnd := end.In(zone)
	civilEnd := localEnd.Format(time.DateOnly)
	if localEnd.Hour() != 0 || localEnd.Minute() != 0 || localEnd.Second() != 0 || localEnd.Nanosecond() != 0 {
		civilEnd = localEnd.AddDate(0, 0, 1).Format(time.DateOnly)
	}
	records, err := readModelRecords(ctx, tx, `SELECT payload FROM records WHERE collection='calendar.event' AND deleted=0 AND coalesce(json_extract(payload,'$.data.all_day'),0)=0 AND julianday(json_extract(payload,'$.data.start'))<=julianday(?) AND julianday(json_extract(payload,'$.data.end'))>=julianday(?)
 UNION ALL SELECT payload FROM records WHERE collection='calendar.event' AND deleted=0 AND json_extract(payload,'$.data.all_day')=1 AND json_extract(payload,'$.data.start')<? AND json_extract(payload,'$.data.end')>? LIMIT 10001`, end.UTC().Format(time.RFC3339Nano), start.UTC().Format(time.RFC3339Nano), civilEnd, civilStart)
	if err != nil {
		return result, err
	}
	if len(records) > 10000 {
		return result, contracts.ErrCapacity
	}
	nodes := []calendarRangeNode{}
	text := strings.ToLower(query.Text)
	for _, record := range records {
		var fields struct {
			Start, End             string
			CalendarID             string `json:"calendar_id"`
			AllDay                 bool   `json:"all_day"`
			Title, Location, Notes string
			Attendees              []struct{ Name, URL string }
		}
		if json.Unmarshal(record.Data, &fields) != nil {
			return result, contracts.ErrCorrupt
		}
		if len(query.Calendars) > 0 && !slices.Contains(query.Calendars, fields.CalendarID) {
			continue
		}
		if text != "" {
			search := fields.Title + " " + fields.Location + " " + fields.Notes
			for _, attendee := range fields.Attendees {
				search += " " + attendee.Name + " " + attendee.URL
			}
			if !strings.Contains(strings.ToLower(search), text) {
				continue
			}
		}
		layout := time.RFC3339Nano
		if fields.AllDay {
			layout = time.DateOnly
		}
		eventStart, x := time.ParseInLocation(layout, fields.Start, zone)
		eventEnd, y := time.ParseInLocation(layout, fields.End, zone)
		if x != nil || y != nil || !eventEnd.After(eventStart) {
			result.CoverageNotes = append(result.CoverageNotes, "An event has an unsupported date range")
			continue
		}
		if !eventStart.Before(end) || !eventEnd.After(start) {
			continue
		}
		nodes = append(nodes, calendarRangeNode{record: record, start: eventStart, end: eventEnd, allDay: fields.AllDay})
	}
	slices.SortFunc(nodes, func(a, b calendarRangeNode) int {
		if a.allDay != b.allDay {
			if a.allDay {
				return -1
			}
			return 1
		}
		if c := a.start.Compare(b.start); c != 0 {
			return c
		}
		if c := b.end.Compare(a.end); c != 0 {
			return c
		}
		return strings.Compare(a.record.ID, b.record.ID)
	})
	groupStart := 0
	var groupEnd time.Time
	columns := []time.Time{}
	finish := func(end int) {
		if end == groupStart {
			return
		}
		group := nodes[groupStart].record.ID
		for _, node := range nodes[groupStart:end] {
			result.Events = append(result.Events, contracts.CalendarRangeEvent{Event: node.record, OverlapGroup: group, Column: node.column, Columns: len(columns)})
		}
	}
	for i := range nodes {
		node := &nodes[i]
		if i == 0 || node.allDay != nodes[i-1].allDay || !node.start.Before(groupEnd) {
			finish(i)
			groupStart = i
			groupEnd = node.end
			columns = nil
		}
		column := 0
		for column < len(columns) && columns[column].After(node.start) {
			column++
		}
		if column == len(columns) {
			columns = append(columns, node.end)
		} else {
			columns[column] = node.end
		}
		node.column = column
		if node.end.After(groupEnd) {
			groupEnd = node.end
		}
	}
	finish(len(nodes))
	order := map[string]time.Time{}
	for _, node := range nodes {
		order[node.record.ID] = node.start
	}
	slices.SortStableFunc(result.Events, func(a, b contracts.CalendarRangeEvent) int {
		if c := order[a.Event.ID].Compare(order[b.Event.ID]); c != 0 {
			return c
		}
		return strings.Compare(a.Event.ID, b.Event.ID)
	})
	s.calendarCache.put(cacheRevision, cacheKey, result)
	return result, nil
}
