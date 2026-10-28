package scenarios

import (
	"context"
	"errors"
	"fmt"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func automaticHealthyChecks(ctx context.Context, w *world.World, evidence *Evidence, add func(string, bool)) bool {
	if evidence.Metrics == nil {
		evidence.Metrics = map[string]any{}
	}
	drained := w.ControlAction("/flush", struct{}{}) == nil
	add("automatic_recovery_capture_drained", drained)
	if !drained {
		return false
	}
	endpoints := append([]world.Endpoint{w.Gateway}, w.Replicas...)
	observers := []*watcher{}
	scoped := []*watcher{}
	evidence.Metrics["automatic_probe_stage"] = "fixture"
	var fixture contracts.ReadResult
	if _, err := world.Request(ctx, w.Gateway, "/v1/op/records.query", contracts.ReadQuery{Collection: "reminders.reminder", Limit: 1}, &fixture); err != nil || len(fixture.Records) != 1 {
		return false
	}
	for i, endpoint := range endpoints {
		evidence.Metrics["automatic_probe_stage"] = fmt.Sprintf("watch_%d", i)
		observer, err := watch(ctx, endpoint, false, func(record contracts.Record) bool { return truthAbsent(w, record) })
		if err != nil {
			return false
		}
		defer observer.close()
		observers = append(observers, observer)
		evidence.Metrics["automatic_probe_stage"] = fmt.Sprintf("token_%d", i)
		token, err := w.ScopedToken(endpoint, "system:read")
		if err != nil {
			return false
		}
		endpoint.Token = token
		evidence.Metrics["automatic_probe_stage"] = fmt.Sprintf("query_%d", i)
		var result contracts.ReadResult
		_, err = world.Request(ctx, endpoint, "/v1/op/records.query", contracts.ReadQuery{Collection: "reminders.reminder"}, &result)
		if err != nil || len(result.Records) != 0 {
			return false
		}
		evidence.Metrics["automatic_probe_stage"] = fmt.Sprintf("record_%d", i)
		status, _ := world.Request(ctx, endpoint, "/v1/records/reminders.reminder/"+fixture.Records[0].ID, nil, nil)
		if status != 404 {
			return false
		}
		evidence.Metrics["automatic_probe_stage"] = fmt.Sprintf("search_%d", i)
		result = contracts.ReadResult{}
		searchStarted := time.Now()
		status, err = world.Request(ctx, endpoint, "/v1/op/search", contracts.ReadQuery{Text: "ab"}, &result)
		evidence.Metrics[fmt.Sprintf("automatic_scoped_search_%d_ms", i)] = float64(time.Since(searchStarted)) / float64(time.Millisecond)
		if (err != nil && status != 404 && status != 422) || (err == nil && len(result.Records) != 0) {
			add(fmt.Sprintf("automatic_scope_search_%d", i), false)
			var timeout interface{ Timeout() bool }
			evidence.Assertions[len(evidence.Assertions)-1].Detail = fmt.Sprintf("http_status=%d timeout=%t context_cancelled=%t records=%d", status, errors.As(err, &timeout) && timeout.Timeout(), ctx.Err() != nil, len(result.Records))
			return false
		}
		add(fmt.Sprintf("automatic_scope_query_search_records_%d", i), true)
		observer, err = watch(ctx, endpoint, true, func(record contracts.Record) bool { return truthAbsent(w, record) })
		if err != nil {
			return false
		}
		defer observer.close()
		scoped = append(scoped, observer)
	}
	evidence.Metrics["automatic_probe_stage"] = "live_probe"
	probe := contracts.NewID("automatic")
	if w.ControlAction("/external-change", map[string]string{"key": probe}) != nil {
		return false
	}
	effect := time.Now()
	if w.Await(10*time.Second, func() bool {
		for _, observer := range observers {
			if _, ok := observer.observed(probe); !ok {
				return false
			}
		}
		return true
	}) != nil {
		return false
	}
	passed := true
	for i, observer := range observers {
		seen, _ := observer.observed(probe)
		latency := max(0, float64(seen.At.Sub(effect))/float64(time.Millisecond))
		endpoint := "effect_done→gateway_visible"
		if i > 0 {
			endpoint = fmt.Sprintf("effect_done→replica_visible:replica%d", i-1)
		}
		evidence.Latency[endpoint] = stats([]float64{latency}, 0)
		observer.mu.Lock()
		valid := !observer.falseDelete && !observer.invalid
		observer.mu.Unlock()
		add(fmt.Sprintf("automatic_metadata_latency_and_no_false_deletion_%d", i), valid && latency < 1000)
		passed = passed && valid && latency < 1000
	}
	if w.Await(5*time.Second, func() bool {
		for _, observer := range scoped {
			observer.mu.Lock()
			advanced := observer.last.Sequence > observer.initialSequence
			observer.mu.Unlock()
			if !advanced {
				return false
			}
		}
		return true
	}) != nil {
		return false
	}
	for i, observer := range scoped {
		valid := observer.valid()
		add(fmt.Sprintf("automatic_scope_sse_watermark_%d", i), valid)
		passed = passed && valid
	}
	delete(evidence.Metrics, "automatic_probe_stage")
	return passed
}
