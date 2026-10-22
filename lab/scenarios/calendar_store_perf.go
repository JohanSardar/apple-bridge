package scenarios

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/simapple"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func calendarRangePerf(ctx context.Context, w *world.World, e *Evidence, add func(string, bool)) {
	records := []contracts.Observation{}
	put := func(collection, key string, data map[string]any) {
		raw, _ := json.Marshal(data)
		records = append(records, contracts.Observation{Kind: "upsert", Collection: collection, NativeKey: key, Data: raw, ObservedAt: time.Now().UTC()})
	}
	put("calendar.calendar", "store-perf", map[string]any{"title": "Bridge Test range perf", "writable": true, "color": "#007aff", "account": "demo"})
	for index := 0; index < 1000; index++ {
		start := time.Date(2026, time.May, 1+index%28, 8+(index/28)%8, (index%4)*15, 0, 0, time.UTC)
		put("calendar.event", fmt.Sprintf("store-perf-%04d", index), map[string]any{"title": fmt.Sprintf("Bridge Test %04d", index), "start": start.Format(time.RFC3339), "end": start.Add(30 * time.Minute).Format(time.RFC3339), "all_day": false, "calendar_id": map[string]any{"$ref": map[string]string{"collection": "calendar.calendar", "native_key": "store-perf"}}})
	}
	add("month_1000_fixture", w.ControlAction("/dataset", simapple.Dataset{Records: records, Blobs: map[string][]byte{}}) == nil)
	add("month_1000_converged", w.Await(10*time.Second, func() bool { return convergence(w) }) == nil)
	query := contracts.CalendarRangeQuery{Start: "2026-05-01T00:00:00Z", End: "2026-06-01T00:00:00Z", TimeZone: "America/New_York"}
	for _, protocol := range []string{"rest", "mcp"} {
		var headers http.Header
		var body []byte
		client := scripted.Client{Endpoint: w.Gateway, Protocol: protocol, Observe: func(h http.Header, b []byte) { headers, body = h, b }}
		var page calendarStorePage
		err := client.Call(ctx, "calendar.range", query, &page)
		add(protocol+"_month_exactly_1000", err == nil && len(page.Events) == 1000)
		if err != nil || len(page.Events) != 1000 {
			continue
		}
		values := []float64{}
		missing := 0
		for index := 0; index < 100; index++ {
			if client.Call(ctx, "calendar.range", query, nil) != nil {
				missing++
				continue
			}
			value, _, known := reminderHandlerTiming(protocol, headers, body)
			if known {
				values = append(values, value)
			} else {
				missing++
			}
		}
		measured := stats(values, missing)
		e.Latency["calendar_range_1000_month_"+protocol+"_gateway"] = measured
		add(protocol+"_month_handler_p95_under_10ms", measured.Samples == 100 && measured.Timeouts == 0 && measured.P95 <= 10)
	}
}
