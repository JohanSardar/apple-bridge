package scenarios

import (
	"context"
	"encoding/json"
	"errors"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func nativeForwardedDelete(ctx context.Context, w *world.World, e *Evidence, record contracts.Record, name string) bool {
	started := time.Now()
	if e.Metrics == nil {
		e.Metrics = map[string]any{}
	}
	e.Metrics[name+"_fixture_version"] = record.Version
	if err := w.Await(3*time.Second, func() bool {
		current, err := w.Records(w.Gateway)
		if err != nil {
			return false
		}
		for _, candidate := range current {
			if candidate.ID == record.ID && candidate.Collection == record.Collection {
				record = candidate
				break
			}
		}
		replica, err := w.Records(w.Replicas[0])
		for _, candidate := range replica {
			if candidate.ID == record.ID && candidate.Collection == record.Collection {
				return err == nil && candidate.Version == record.Version
			}
		}
		return false
	}); err != nil {
		return false
	}
	e.Metrics[name+"_submitted_version"] = record.Version
	client := scripted.Client{Endpoint: w.Replicas[0], Protocol: "rest"}
	request := map[string]any{"command_id": contracts.NewCommandID(), "args": map[string]any{"id": record.ID}, "expected_version": record.Version, "wait_ms": 15000}
	var result map[string]any
	commandStarted := time.Now()
	e.Metrics[name+"_request_setup_ms"] = float64(commandStarted.Sub(started)) / float64(time.Millisecond)
	err := client.Call(ctx, "reminders.delete", request, &result)
	e.Latency[name] = stats([]float64{float64(time.Since(commandStarted)) / float64(time.Millisecond)}, 0)
	var problem *contracts.Error
	if errors.As(err, &problem) {
		e.Metrics[name+"_error_code"] = problem.Code
	}
	e.Metrics[name+"_disposition"] = result["disposition"]
	if err != nil {
		states := []map[string]any{}
		current, _ := w.Records(w.Gateway)
		for _, record := range current {
			var data map[string]any
			if record.Collection == "system.source" && json.Unmarshal(record.Data, &data) == nil && data["source"] != nil {
				states = append(states, map[string]any{"source": data["source"], "state": data["state"], "reason": data["reason"]})
			}
		}
		e.Metrics[name+"_source_states"] = states
	}
	return err == nil && result["disposition"] == "locally_observed"
}

func nativeBlobStage(w *world.World, e *Evidence, name string) bool {
	started := time.Now()
	var fixture map[string]any
	err := w.BlobControl(map[string]string{"action": "relay-fixture"}, &fixture)
	e.Latency[name] = stats([]float64{float64(time.Since(started)) / float64(time.Millisecond)}, 0)
	return err == nil
}
