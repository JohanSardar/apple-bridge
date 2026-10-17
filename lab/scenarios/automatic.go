package scenarios

import (
	"context"
	"fmt"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func automaticHealthyChecks(ctx context.Context, w *world.World, evidence *Evidence, add func(string, bool)) bool {
	if evidence.Metrics == nil {
		evidence.Metrics = map[string]any{}
	}
	check := func(name string, ok bool, detail string) bool {
		add(name, ok)
		if !ok {
			evidence.Metrics["automatic_failure"] = name
			evidence.Assertions[len(evidence.Assertions)-1].Detail = detail
		}
		return ok
	}
	evidence.Metrics["automatic_phase"] = "flush"
	err := w.ControlAction("/flush", struct{}{})
	if !check("automatic_recovery_capture_drained", err == nil, fmt.Sprintf("error_type=%T", err)) {
		return false
	}
	endpoints := append([]world.Endpoint{w.Gateway}, w.Replicas...)
	observers := []*watcher{}
	scoped := []*watcher{}
	evidence.Metrics["automatic_phase"] = "fixture"
	var fixture contracts.ReadResult
	status, err := world.Request(ctx, w.Gateway, "/v1/op/records.query", contracts.ReadQuery{Collection: "reminders.reminder", Limit: 1}, &fixture)
	if !check("automatic_fixture_available", err == nil && len(fixture.Records) == 1, fmt.Sprintf("status=%d error_type=%T records=%d", status, err, len(fixture.Records))) {
		return false
	}
	for i, endpoint := range endpoints {
		evidence.Metrics["automatic_phase"] = fmt.Sprintf("watch_%d", i)
		observer, err := watch(ctx, endpoint, false, func(record contracts.Record) bool { return truthAbsent(w, record) })
		if !check(fmt.Sprintf("automatic_sse_connected_%d", i), err == nil, fmt.Sprintf("error_type=%T", err)) {
			return false
		}
		defer observer.close()
		observers = append(observers, observer)
		token, err := w.ScopedToken(endpoint, "system:read")
		if !check(fmt.Sprintf("automatic_scoped_token_%d", i), err == nil, fmt.Sprintf("error_type=%T", err)) {
			return false
		}
		endpoint.Token = token
		var result contracts.ReadResult
		evidence.Metrics["automatic_phase"] = fmt.Sprintf("query_%d", i)
		status, err = world.Request(ctx, endpoint, "/v1/op/records.query", contracts.ReadQuery{Collection: "reminders.reminder"}, &result)
		if !check(fmt.Sprintf("automatic_scope_query_%d", i), err == nil && len(result.Records) == 0, fmt.Sprintf("status=%d error_type=%T records=%d", status, err, len(result.Records))) {
			return false
		}
		status, err = world.Request(ctx, endpoint, "/v1/records/reminders.reminder/"+fixture.Records[0].ID, nil, nil)
		if !check(fmt.Sprintf("automatic_scope_record_%d", i), status == 404, fmt.Sprintf("status=%d error_type=%T", status, err)) {
			return false
		}
		result = contracts.ReadResult{}
		evidence.Metrics["automatic_phase"] = fmt.Sprintf("search_%d", i)
		status, err = world.Request(ctx, endpoint, "/v1/op/search", contracts.ReadQuery{Text: "ab"}, &result)
		allowed := (err != nil && (status == 404 || status == 422)) || (err == nil && len(result.Records) == 0)
		if !check(fmt.Sprintf("automatic_scope_search_%d", i), allowed, fmt.Sprintf("status=%d error_type=%T records=%d", status, err, len(result.Records))) {
			return false
		}
		add(fmt.Sprintf("automatic_scope_query_search_records_%d", i), true)
		observer, err = watch(ctx, endpoint, true, func(record contracts.Record) bool { return truthAbsent(w, record) })
		if !check(fmt.Sprintf("automatic_scope_sse_connected_%d", i), err == nil, fmt.Sprintf("error_type=%T", err)) {
			return false
		}
		defer observer.close()
		scoped = append(scoped, observer)
	}
	probe := contracts.NewID("automatic")
	evidence.Metrics["automatic_phase"] = "external_change"
	err = w.ControlAction("/external-change", map[string]string{"key": probe})
	if !check("automatic_probe_capture_requested", err == nil, fmt.Sprintf("error_type=%T", err)) {
		return false
	}
	effect := time.Now()
	evidence.Metrics["automatic_phase"] = "observe_probe"
	err = w.Await(10*time.Second, func() bool {
		for _, observer := range observers {
			if _, ok := observer.observed(probe); !ok {
				return false
			}
		}
		return true
	})
	passed := err == nil
	for i, observer := range observers {
		seen, ok := observer.observed(probe)
		observer.mu.Lock()
		invalid, falseDelete, last, initial := observer.invalid, observer.falseDelete, observer.last.Sequence, observer.initialSequence
		observer.mu.Unlock()
		detail := fmt.Sprintf("initial_sequence=%d last_sequence=%d invalid_cursor=%t false_delete=%t", initial, last, invalid, falseDelete)
		passed = check(fmt.Sprintf("automatic_probe_seen_%d", i), ok, detail) && passed
		passed = check(fmt.Sprintf("automatic_cursor_continuity_%d", i), !invalid, detail) && passed
		passed = check(fmt.Sprintf("automatic_no_false_deletion_%d", i), !falseDelete, detail) && passed
		if ok {
			latency := max(0, float64(seen.At.Sub(effect))/float64(time.Millisecond))
			endpoint := "effect_done→gateway_visible"
			if i > 0 {
				endpoint = fmt.Sprintf("effect_done→replica_visible:replica%d", i-1)
			}
			evidence.Latency[endpoint] = stats([]float64{latency}, 0)
			passed = check(fmt.Sprintf("automatic_metadata_latency_and_no_false_deletion_%d", i), !invalid && !falseDelete && latency < 1000, fmt.Sprintf("%s latency_ms=%.3f limit_ms=1000", detail, latency)) && passed
		}
	}
	if !passed {
		return false
	}
	evidence.Metrics["automatic_phase"] = "scoped_watermarks"
	err = w.Await(5*time.Second, func() bool {
		for _, observer := range scoped {
			observer.mu.Lock()
			advanced := observer.last.Sequence > observer.initialSequence
			observer.mu.Unlock()
			if !advanced {
				return false
			}
		}
		return true
	})
	passed = err == nil
	for i, observer := range scoped {
		observer.mu.Lock()
		valid, initial, last := !observer.invalid, observer.initialSequence, observer.last.Sequence
		observer.mu.Unlock()
		passed = check(fmt.Sprintf("automatic_scope_sse_watermark_%d", i), valid && last > initial, fmt.Sprintf("initial_sequence=%d last_sequence=%d invalid_cursor=%t", initial, last, !valid)) && passed
	}
	if passed {
		evidence.Metrics["automatic_phase"] = "complete"
	}
	return passed
}
