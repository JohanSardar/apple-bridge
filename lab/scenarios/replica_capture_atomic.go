package scenarios

import (
	"context"
	"encoding/json"
	"fmt"
	"os/exec"
	"slices"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func runReplicaCaptureAtomic(ctx context.Context, s Scenario, level string) Evidence {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e := Evidence{RunID: contracts.NewID("atomic"), Scenario: s.ID, Level: level, GitSHA: strings.TrimSpace(string(sha)), At: time.Now().UTC(), Assertions: []Assertion{}, Latency: map[string]Latency{}, Hosts: []string{"air"}}
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	w, err := world.New(ctx, level, 1, 0)
	if err != nil {
		add("world_ready", false)
		return e
	}
	defer w.Close()
	if w.Await(10*time.Second, func() bool { return convergence(w) }) != nil {
		add("initial_convergence", false)
		return e
	}
	calendar := func(action, key string, data map[string]any) error {
		return w.ControlAction("/calendar", map[string]any{"action": action, "key": key, "data": data})
	}
	if calendar("calendar", "test-calendar", nil) != nil || calendar("create", "atomic-series", map[string]any{"title": "[ab-test atomic] original", "start": "2026-11-01", "end": "2026-11-02", "all_day": true, "recurrence": []any{map[string]any{"frequency": "daily", "interval": 1, "count": 3}}}) != nil {
		add("calendar_fixture", false)
		return e
	}
	if w.Await(10*time.Second, func() bool { return convergence(w) }) != nil {
		add("calendar_convergence", false)
		return e
	}
	for _, phase := range []string{"generic", "generic-byte", "calendar"} {
		if w.Await(10*time.Second, func() bool {
			g, _ := w.Cursor(w.Gateway)
			r, _ := w.Cursor(w.Replicas[0])
			return g.Sequence == r.Sequence
		}) != nil {
			add(phase+"_caught_up", false)
			return e
		}
		r, _ := w.Cursor(w.Replicas[0])
		w.Partition(0)
		goal := uint64(127)
		if phase == "generic-byte" {
			goal = 1
		}
		if phase == "calendar" {
			goal = 124
		}
		aligned := false
		for n := 0; n < 256; n++ {
			g, _ := w.Cursor(w.Gateway)
			if (g.Sequence-r.Sequence)%128 == goal {
				aligned = true
				break
			}
			padding := map[string]any{"key": fmt.Sprintf("padding-%s-%d", phase, n)}
			if phase == "generic-byte" {
				padding["data"] = map[string]any{"title": "[ab-test] byte padding", "notes": strings.Repeat("p", 100000)}
			}
			if w.ControlAction("/external-change", padding) != nil || w.FlushCaptures(5*time.Second) != nil {
				break
			}
		}
		add(phase+"_capture_crosses_batch_boundary", aligned)
		if !aligned {
			return e
		}
		signature := func(endpoint world.Endpoint) string {
			values := []string{}
			all, _ := w.Records(endpoint)
			for _, record := range all {
				if (phase == "generic" && strings.HasPrefix(recordTitle(record), "[ab-test] seed_") || phase == "generic-byte" && strings.HasPrefix(recordTitle(record), "[ab-test atomic-byte]")) && record.Collection == "reminders.reminder" || phase == "calendar" && record.Collection == "calendar.event" && strings.HasPrefix(recordTitle(record), "[ab-test atomic]") {
					values = append(values, record.ID+"/"+string(record.Data))
				}
			}
			slices.Sort(values)
			return strings.Join(values, "\n")
		}
		initial := signature(w.Replicas[0])
		before, _ := w.Cursor(w.Gateway)
		observer, err := watch(ctx, w.Replicas[0], false, func(contracts.Record) bool { return true })
		if err != nil {
			add(phase+"_watch", false)
			return e
		}
		var target contracts.Record
		if phase == "generic" {
			err = w.ControlAction("/seed", map[string]any{"count": 3})
		} else if phase == "generic-byte" {
			observations := []contracts.Observation{}
			for n := 0; n < 3; n++ {
				key := fmt.Sprintf("atomic-byte-%d", n)
				data := mustJSON(map[string]any{"title": "[ab-test atomic-byte] " + key, "notes": strings.Repeat("b", 60000)})
				if err = w.ControlAction("/external-change", map[string]any{"key": key, "data": json.RawMessage(data), "omit_capture": true}); err != nil {
					break
				}
				observations = append(observations, contracts.Observation{Kind: "upsert", Collection: "reminders.reminder", NativeKey: key, Data: data, ObservedAt: time.Now().UTC()})
			}
			if err == nil {
				err = w.ControlAction("/capture", contracts.Capture{Source: contracts.Reminders, Observations: observations})
			}
		} else {
			all, _ := w.Records(w.Gateway)
			for _, record := range all {
				var data map[string]any
				json.Unmarshal(record.Data, &data)
				if record.Collection == "calendar.event" && data["start"] == "2026-11-02" {
					target = record
					break
				}
			}
			client := scripted.Client{Endpoint: w.Gateway, Protocol: "rest"}
			var outcome contracts.CommandRecord
			err = client.Call(ctx, "calendar.update_event", contracts.Command{ID: contracts.NewCommandID(), ExpectedVersion: target.Version, WaitMS: 15000, Args: mustJSON(map[string]any{"id": target.ID, "span": "future", "start": "2026-11-04", "end": "2026-11-05"})}, &outcome)
			add("calendar_split_observed", err == nil && outcome.Disposition == contracts.DispositionLocallyObserved)
		}
		if err != nil || w.FlushCaptures(5*time.Second) != nil {
			observer.close()
			add(phase+"_capture", false)
			return e
		}
		through, _ := w.Cursor(w.Gateway)
		final := signature(w.Gateway)
		readsAtomic := true
		w.Heal(0)
		caught := w.Await(10*time.Second, func() bool {
			value := signature(w.Replicas[0])
			readsAtomic = readsAtomic && (value == initial || value == final)
			observer.mu.Lock()
			defer observer.mu.Unlock()
			return observer.last.Sequence >= through.Sequence
		}) == nil
		observer.mu.Lock()
		batches := map[time.Time]int{}
		for _, seen := range observer.changes {
			if seen.Change.Sequence <= before.Sequence || seen.Change.Record.Collection == "commands.command" {
				continue
			}
			if phase != "calendar" && seen.Change.Record.Collection != "reminders.reminder" {
				continue
			}
			if phase == "calendar" && seen.Change.Record.Collection != "calendar.event" {
				continue
			}
			batches[seen.At]++
		}
		observer.mu.Unlock()
		observer.close()
		e.Assertions = append(e.Assertions, Assertion{Name: phase + "_capture_never_partial_in_replica_live_delta", Passed: caught && len(batches) == 1, Detail: fmt.Sprintf("capture_batches=%d after=%d through=%d replica_before=%d", len(batches), before.Sequence, through.Sequence, r.Sequence)})
		add(phase+"_replica_reads_never_partial", readsAtomic)
		add(phase+"_replica_converges", convergence(w))
	}
	e.Passed = true
	for _, a := range e.Assertions {
		e.Passed = e.Passed && a.Passed
	}
	return e
}
