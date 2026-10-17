package operations

import (
	"context"
	"encoding/json"
	"slices"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

func (g *LocalGateway) calendarSeriesKey(ctx context.Context, p trust.Principal, id, calendar string) string {
	if id != "" {
		result, err := g.State.Read(ctx, p.Visibility(), contracts.ReadQuery{Collection: "calendar.event", ID: id})
		if err == nil && len(result.Records) == 1 {
			var data struct {
				Series   string `json:"series_id"`
				Calendar string `json:"calendar_id"`
			}
			if json.Unmarshal(result.Records[0].Data, &data) == nil && data.Series != "" {
				return "calendar-series:" + data.Calendar + ":" + data.Series
			}
		}
		return id
	}
	return "calendar:" + calendar
}
func (s *Service) agenda(ctx context.Context, p trust.Principal, input json.RawMessage) (contracts.ReadResult, error) {
	var q contracts.AgendaQuery
	if contracts.DecodeInput(input, &q) != nil {
		return contracts.ReadResult{}, contracts.ErrInvalid
	}
	start, err := time.Parse(time.RFC3339Nano, q.Start)
	end, endErr := time.Parse(time.RFC3339Nano, q.End)
	if err != nil || endErr != nil || !end.After(start) {
		return contracts.ReadResult{}, contracts.ErrInvalid
	}
	result := contracts.ReadResult{Records: []contracts.Record{}}
	after := ""
	for {
		page, err := s.State.Read(ctx, p.Visibility(), contracts.ReadQuery{Collection: "calendar.event", After: after, Limit: 10000})
		if err != nil {
			return result, err
		}
		result.Cursor = page.Cursor
		for _, record := range page.Records {
			var data struct {
				Start    string `json:"start"`
				End      string `json:"end"`
				Calendar string `json:"calendar_id"`
				AllDay   bool   `json:"all_day"`
			}
			if json.Unmarshal(record.Data, &data) != nil {
				return result, contracts.ErrCorrupt
			}
			overlaps := false
			if data.AllDay {
				endDay := end.Format(time.DateOnly)
				if end.Hour() != 0 || end.Minute() != 0 || end.Second() != 0 || end.Nanosecond() != 0 {
					endDay = end.AddDate(0, 0, 1).Format(time.DateOnly)
				}
				overlaps = data.Start < endDay && data.End > start.Format(time.DateOnly)
			} else {
				a, e := time.Parse(time.RFC3339Nano, data.Start)
				b, f := time.Parse(time.RFC3339Nano, data.End)
				overlaps = e == nil && f == nil && a.Before(end) && b.After(start)
			}
			if overlaps && (q.CalendarID == "" || q.CalendarID == data.Calendar) {
				result.Records = append(result.Records, record)
			}
		}
		if page.Next == "" {
			break
		}
		after = page.Next
	}
	slices.SortFunc(result.Records, func(a, b contracts.Record) int {
		var av, bv map[string]any
		json.Unmarshal(a.Data, &av)
		json.Unmarshal(b.Data, &bv)
		as, _ := av["start"].(string)
		bs, _ := bv["start"].(string)
		if cmp := strings.Compare(as, bs); cmp != 0 {
			return cmp
		}
		return strings.Compare(a.ID, b.ID)
	})
	return result, nil
}
