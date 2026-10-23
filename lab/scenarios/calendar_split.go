package scenarios

import (
	"context"
	"encoding/json"
	"fmt"
	"slices"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/lab/world"
)

func watchCalendarSplit(ctx context.Context, w *world.World, tag string, diagnostics map[string]any) func() bool {
	endpoints := append([]world.Endpoint{w.Gateway}, w.Replicas...)
	snapshot := func(endpoint world.Endpoint) string {
		records, err := w.Records(endpoint)
		if err != nil {
			return "read_failed"
		}
		identities := []string{}
		for _, record := range records {
			if record.Collection != "calendar.event" || !strings.HasPrefix(recordTitle(record), tag) {
				continue
			}
			var data map[string]any
			if json.Unmarshal(record.Data, &data) != nil {
				return "decode_failed"
			}
			identities = append(identities, fmt.Sprint(data["series_id"], "|", data["occurrence_date"], "|", data["start"], "|", data["title"]))
		}
		slices.Sort(identities)
		return strings.Join(identities, "\n")
	}
	before := snapshot(w.Gateway)
	mismatched := map[int]bool{}
	ready := before != "read_failed" && before != "decode_failed" && w.Await(5*time.Second, func() bool {
		matched := true
		for index, endpoint := range endpoints {
			if snapshot(endpoint) != before {
				mismatched[index] = true
				matched = false
			}
		}
		return matched
	}) == nil
	indices := []int{}
	for index := range endpoints {
		if mismatched[index] {
			indices = append(indices, index)
		}
	}
	diagnostics["split_baseline_ready"] = ready
	diagnostics["split_baseline_initially_mismatched_endpoints"] = indices
	if !ready {
		return func() bool { return false }
	}
	stop, done := make(chan struct{}), make(chan []string, 1)
	go func() {
		observed := []string{}
		ticker := time.NewTicker(10 * time.Millisecond)
		defer ticker.Stop()
		for {
			for _, endpoint := range endpoints {
				observed = append(observed, snapshot(endpoint))
			}
			select {
			case <-ctx.Done():
				done <- append(observed, "context_expired")
				return
			case <-stop:
				done <- observed
				return
			case <-ticker.C:
			}
		}
	}()
	return func() bool {
		after := snapshot(w.Gateway)
		converged := w.Await(5*time.Second, func() bool {
			for _, endpoint := range endpoints {
				if snapshot(endpoint) != after {
					return false
				}
			}
			return true
		}) == nil
		close(stop)
		for _, value := range <-done {
			if value != before && value != after {
				return false
			}
		}
		return converged
	}
}
