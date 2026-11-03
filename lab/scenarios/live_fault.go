package scenarios

import (
	"context"
	"encoding/json"
	"fmt"
	"os/exec"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

type staleOutcome struct {
	StaleFence   uint64          `json:"stale_fence"`
	CurrentFence uint64          `json:"current_fence"`
	Reply        string          `json:"reply"`
	Key          string          `json:"key"`
	Collection   string          `json:"collection"`
	Data         json.RawMessage `json:"data"`
}

func liveFaultEvidence(s Scenario, level string) Evidence {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	return Evidence{RunID: contracts.NewID("live-fault"), Scenario: s.ID, Level: level, GitSHA: strings.TrimSpace(string(sha)), At: time.Now().UTC(), Assertions: []Assertion{}, Latency: map[string]Latency{}, Hosts: []string{"air"}}
}

func finishLiveFault(e Evidence) Evidence {
	e.Passed = len(e.Assertions) > 0
	for _, a := range e.Assertions {
		e.Passed = e.Passed && a.Passed
	}
	return e
}

func recordVisible(ctx context.Context, w *world.World, collection string, data json.RawMessage) (bool, error) {
	var result contracts.ReadResult
	if _, err := world.Request(ctx, w.Gateway, "/v1/op/records.query", contracts.ReadQuery{Collection: collection, Limit: 1000}, &result); err != nil {
		return false, err
	}
	var want any
	if json.Unmarshal(data, &want) != nil {
		return false, contracts.ErrInvalid
	}
	wantJSON, _ := json.Marshal(want)
	for _, record := range result.Records {
		var got any
		if json.Unmarshal(record.Data, &got) != nil || record.Deleted {
			continue
		}
		if gotJSON, _ := json.Marshal(got); string(gotJSON) == string(wantJSON) {
			return true, nil
		}
	}
	return false, nil
}

func runStaleFence(ctx context.Context, s Scenario, level string) Evidence {
	e := liveFaultEvidence(s, level)
	add := func(name string, ok bool, detail string) {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok, Detail: detail})
	}
	w, err := world.New(ctx, level, 0, 0)
	if err != nil {
		add("world_start", false, err.Error())
		return finishLiveFault(e)
	}
	defer w.Close()
	baseline := map[string]any{"key": "stale-fence-baseline", "collection": "reminders.reminder", "data": map[string]any{"title": "[ab-test] fence baseline", "completed": false}}
	if err = w.ControlAction("/external-change", baseline); err != nil {
		add("action", false, err.Error())
		return finishLiveFault(e)
	}
	if err = w.FlushCaptures(15 * time.Second); err != nil {
		add("action", false, err.Error())
		return finishLiveFault(e)
	}
	input := map[string]any{"key": "stale-fence-record", "collection": "reminders.reminder", "data": map[string]any{"title": "[ab-test] frame on superseded connection", "completed": false}}
	var outcome staleOutcome
	if err = w.ControlResultWithin("/stale-frame", input, &outcome, 20*time.Second); err != nil {
		add("action", false, err.Error())
		return finishLiveFault(e)
	}
	visible, err := recordVisible(ctx, w, outcome.Collection, outcome.Data)
	if err != nil {
		add("action", false, err.Error())
		return finishLiveFault(e)
	}
	superseded := outcome.StaleFence > 0 && outcome.StaleFence < outcome.CurrentFence
	add("stale_fence_rejected", superseded && outcome.Key != "" && outcome.Reply != "committed" && !visible, fmt.Sprintf("superseded=%t stale_reply=%s visible_through_public_query=%t", superseded, outcome.Reply, visible))
	return finishLiveFault(e)
}

func runAckDurable(ctx context.Context, s Scenario, level string) Evidence {
	e := liveFaultEvidence(s, level)
	add := func(name string, ok bool, detail string) {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok, Detail: detail})
	}
	w, err := world.New(world.WithoutAcknowledgedCapture(ctx), level, 0, 0)
	if err != nil {
		add("world_start", false, err.Error())
		return finishLiveFault(e)
	}
	defer w.Close()
	data := map[string]any{"title": "[ab-test] acked data must be durable", "completed": false}
	if err = w.ControlAction("/external-change", map[string]any{"key": "ack-durable-record", "collection": "reminders.reminder", "data": data}); err != nil {
		add("action", false, err.Error())
		return finishLiveFault(e)
	}
	type stats struct {
		Revision  map[string]uint64 `json:"revision"`
		Committed map[string]uint64 `json:"committed"`
	}
	acked := false
	err = w.Await(15*time.Second, func() bool {
		var current stats
		response, err := w.Control.Get("http://sim/stats")
		if err != nil {
			return false
		}
		defer response.Body.Close()
		if json.NewDecoder(response.Body).Decode(&current) != nil {
			return false
		}
		acked = current.Revision["reminders"] > 0 && current.Committed["reminders"] >= current.Revision["reminders"]
		return acked
	})
	if err != nil || !acked {
		add("peer_ack_observed", false, "peer never saw Committed for the change")
		return finishLiveFault(e)
	}
	encoded, _ := json.Marshal(data)
	visible, err := recordVisible(ctx, w, "reminders.reminder", encoded)
	if err != nil {
		add("action", false, err.Error())
		return finishLiveFault(e)
	}
	add("acked_data_visible", visible, fmt.Sprintf("peer_acked=%t visible_through_public_query=%t", acked, visible))
	return finishLiveFault(e)
}
