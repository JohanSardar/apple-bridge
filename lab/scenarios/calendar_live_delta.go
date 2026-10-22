package scenarios

import (
	"context"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func startCalendarLiveDelta(ctx context.Context, w *world.World, e *Evidence, observers []*watcher) func() bool {
	e.Conditions = calendarHostConditions()
	tag := "[ab-test " + e.RunID + "]"
	finish := watchCalendarSplit(ctx, w, tag)
	done := make(chan bool, 1)
	var result struct {
		Bytes int `json:"capture_bytes"`
	}
	go func() {
		err := w.ControlAction("/fault", map[string]any{"name": "calendar-part-delay"})
		if err == nil {
			err = w.ControlAction("/calendar", map[string]any{"action": "calendar", "key": "test-calendar"})
		}
		if err == nil {
			err = w.ControlResult("/calendar", map[string]any{"action": "create_many", "key": e.RunID, "count": 100, "data": map[string]any{"title": tag, "notes": strings.Repeat("x", 1600), "start": "2026-11-01T10:00:00Z", "end": "2026-11-01T11:00:00Z"}}, &result)
		}
		done <- err == nil
	}()
	return func() bool {
		ok := <-done
		if e.Metrics == nil {
			e.Metrics = map[string]any{}
		}
		e.Metrics["calendar_live_delta_bytes"] = result.Bytes
		ok = ok && result.Bytes >= 190000 && result.Bytes <= 240000
		visible := w.Await(5*time.Second, func() bool {
			for _, observer := range observers {
				observer.mu.Lock()
				count := 0
				for _, seen := range observer.changes {
					if seen.Change.Record.Source == contracts.Calendar && strings.HasPrefix(recordTitle(seen.Change.Record), tag) {
						count++
					}
				}
				observer.mu.Unlock()
				if count != 100 {
					return false
				}
			}
			return true
		}) == nil
		for _, observer := range observers {
			observer.mu.Lock()
			batches := map[time.Time]bool{}
			for _, seen := range observer.changes {
				if seen.Change.Record.Source == contracts.Calendar && strings.HasPrefix(recordTitle(seen.Change.Record), tag) {
					batches[seen.At] = true
				}
			}
			observer.mu.Unlock()
			ok = ok && len(batches) == 1
		}
		return finish() && visible && ok
	}
}
