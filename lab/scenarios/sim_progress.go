package scenarios

import (
	"bytes"
	"context"
	"database/sql"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

type simCaptureProgress struct {
	Checkpoints map[contracts.Source]string `json:"checkpoints"`
	InProgress  map[contracts.Source]bool   `json:"in_progress"`
	Committed   map[contracts.Source]uint64 `json:"committed"`
	Revision    map[contracts.Source]uint64 `json:"revision"`
}

func runSimProgressRecovery(ctx context.Context, s Scenario, level string) Evidence {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e := Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, GitSHA: string(bytes.TrimSpace(sha)), At: time.Now().UTC(), Hosts: []string{"air"}, Metrics: map[string]any{}, Conditions: map[string]string{"dataset": "512 synthetic messages and 500 synthetic chats", "boundary": "full capture paused after multiple intermediate ACKs, then SimApple process restart"}}
	add := func(name string, passed bool) {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: passed})
	}
	w, err := world.New(ctx, "sim", 1, 0, world.BlobOptions{Scope: "messages:read,system:read"})
	if err != nil {
		add("world_start", false)
		return e
	}
	defer w.Close()
	w.Control.Timeout = 30 * time.Second
	if err = w.FlushCaptures(30 * time.Second); err != nil {
		add("initial_captures_durable", false)
		return e
	}
	add("initial_captures_durable", true)
	status := func() (int, bool, error) {
		response, problem := w.Control.Get("http://sim/messages-status")
		if problem != nil {
			return 0, false, problem
		}
		defer response.Body.Close()
		var result struct {
			Count   int  `json:"capture_count"`
			Blocked bool `json:"bulk_blocked"`
		}
		problem = json.NewDecoder(response.Body).Decode(&result)
		return result.Count, result.Blocked, problem
	}
	before, _, err := status()
	if err != nil {
		add("initial_capture_count", false)
		return e
	}
	if err = w.ControlAction("/messages", map[string]any{"action": "bulk-pause", "key": "rm-message-000128"}); err != nil {
		add("pause_middle_capture_chunk", false)
		return e
	}
	done := make(chan error, 1)
	go func() { done <- w.ControlAction("/readmodels", map[string]any{"action": "seed", "count": 512}) }()
	err = w.Await(30*time.Second, func() bool {
		count, blocked, problem := status()
		return problem == nil && blocked && count >= before+2
	})
	add("multiple_chunks_acknowledged_before_pause", err == nil)
	if err != nil {
		return e
	}
	db, err := sql.Open("sqlite", "file:"+filepath.Join(w.Gateway.Home, "state.db")+"?_pragma=busy_timeout(5000)")
	if err != nil {
		add("gateway_checkpoint_inspection", false)
		return e
	}
	defer db.Close()
	checkpoint := func() (string, error) {
		var value string
		problem := db.QueryRowContext(ctx, "SELECT checkpoint FROM checkpoints WHERE source='messages'").Scan(&value)
		return value, problem
	}
	progress := func(name string) (simCaptureProgress, error) {
		var result simCaptureProgress
		raw, problem := os.ReadFile(filepath.Join(w.Dir, name))
		if problem == nil {
			problem = json.Unmarshal(raw, &result)
		}
		return result, problem
	}
	gateway, err := checkpoint()
	persisted, readErr := progress("sim-progress.json")
	desired, stateErr := progress("sim-state.json")
	add("durable_start_marker_retains_unfinished_revision", err == nil && readErr == nil && stateErr == nil && persisted.InProgress[contracts.Messages] && persisted.Committed[contracts.Messages] < desired.Revision[contracts.Messages])
	add("intermediate_acks_do_not_force_progress_fsync", readErr == nil && gateway != "" && persisted.Checkpoints[contracts.Messages] != gateway)
	if err = w.RestartHelper(); err != nil {
		add("helper_restarts_from_durable_start", false)
		return e
	}
	add("helper_restarts_from_durable_start", true)
	select {
	case <-done:
	case <-time.After(5 * time.Second):
		add("interrupted_request_finishes", false)
		return e
	}
	err = w.FlushCaptures(30 * time.Second)
	add("replayed_capture_finishes", err == nil)
	if err != nil {
		return e
	}
	add("gateway_and_replica_equal_source_truth", w.Await(15*time.Second, func() bool { return convergence(w) }) == nil)
	records, err := w.Records(w.Gateway)
	messages, chats := 0, 0
	versions := err == nil
	for _, record := range records {
		if record.Collection == "messages.message" {
			messages++
		}
		if record.Collection == "messages.chat" {
			chats++
		}
		if record.Collection != "system.source" && record.Version != 1 {
			versions = false
		}
	}
	e.Metrics["messages"] = messages
	e.Metrics["chats"] = chats
	add("all_512_messages_and_500_chats_retained", messages == 512 && chats == 500)
	add("replay_does_not_duplicate_record_versions", versions)
	gateway, err = checkpoint()
	persisted, readErr = progress("sim-progress.json")
	add("final_checkpoint_and_revision_are_durable", err == nil && readErr == nil && !persisted.InProgress[contracts.Messages] && persisted.Checkpoints[contracts.Messages] == gateway && persisted.Committed[contracts.Messages] >= desired.Revision[contracts.Messages])
	e.Passed = true
	for _, assertion := range e.Assertions {
		if !assertion.Passed {
			e.Passed = false
		}
	}
	return e
}
