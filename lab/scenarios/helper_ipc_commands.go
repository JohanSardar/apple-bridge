package scenarios

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"net"
	"os"
	"path/filepath"
	"syscall"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/applepeer"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

type helperCommandResult struct {
	record contracts.CommandRecord
	err    error
}

func helperCommandOrdering(ctx context.Context, w *world.World, conn *net.UnixConn, wire *applepeer.Wire, checkpoint contracts.Checkpoint, e *Evidence) bool {
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	if e.Metrics == nil {
		e.Metrics = map[string]any{}
	}
	list, err := helperIPCRecord(ctx, w, "reminders.list", "Bridge Test")
	add("causal_command_list_available", err == nil && checkpoint.Generation != "")
	if err != nil || checkpoint.Generation == "" {
		return false
	}
	title := "[ab-test " + contracts.NewID("ipc") + "]"
	native := contracts.NewID("native")
	observation := func(value string) contracts.Observation {
		raw, _ := json.Marshal(map[string]any{"title": value, "completed": false, "list_id": map[string]any{"$ref": map[string]string{"collection": "reminders.list", "native_key": "list"}}})
		return contracts.Observation{Kind: "upsert", Collection: "reminders.reminder", NativeKey: native, Data: raw, ObservedAt: time.Now().UTC()}
	}
	create := helperIPCSubmit(ctx, w, "reminders.create", map[string]any{"title": title, "list_id": list.ID}, 0)
	prepare, err := helperIPCFrame(conn, wire, "Prepare")
	if err == nil {
		_, err = wire.Send(contracts.IPCFrame{T: "Prepared", CommandID: prepare.CommandID, Ticket: contracts.NewID("ticket"), Fingerprint: prepare.RequestHash})
	}
	if err == nil {
		_, err = helperIPCFrame(conn, wire, "Dispatch")
	}
	add("causal_create_dispatched", err == nil)
	if err != nil {
		return false
	}
	first := contracts.Capture{Source: contracts.Reminders, Generation: checkpoint.Generation, Predecessor: checkpoint.Checkpoint, Checkpoint: contracts.NewID("cp"), Observations: []contracts.Observation{observation(title)}}
	release, err := helperIPCHold(w, wire, first)
	add("observed_capture_held_before_receipt", err == nil)
	if err != nil {
		return false
	}
	defer release()
	evidence, _ := json.Marshal(map[string]any{"native_key": native, "capture_checkpoint": first.Checkpoint, "fingerprint": prepare.RequestHash, "observed": true})
	_, err = wire.Send(contracts.IPCFrame{T: "Outcome", CommandID: prepare.CommandID, Disposition: "locally_observed", Evidence: evidence})
	if err == nil {
		_, err = wire.Send(contracts.IPCFrame{T: "Ping"})
	}
	if err == nil {
		err = helperPong(conn, wire, time.Now().Add(time.Second))
	}
	add("causal_control_progresses_before_receipt", err == nil)
	if err != nil {
		return false
	}
	early := helperIPCEarly(create)
	add("observed_reply_waits_for_capture_receipt", early == nil)
	if err = release(); err != nil {
		return false
	}
	_, err = helperIPCFrame(conn, wire, "Committed")
	if err != nil {
		return false
	}
	result := helperIPCResult(create, early)
	e.Metrics["ipc_observed_response"] = map[string]any{"state": result.record.State, "disposition": result.record.Disposition, "commit_seq": result.record.CommitSeq, "record_id_count": len(result.record.ObservedRecordIDs)}
	record, readErr := helperIPCRecord(ctx, w, "reminders.reminder", title)
	observed := result.err == nil && result.record.State == "observed" && result.record.Disposition == "locally_observed" && result.record.CommitSeq > 0 && len(result.record.ObservedRecordIDs) == 1 && readErr == nil && result.record.ObservedRecordIDs[0] == record.ID
	add("cited_capture_produces_observed_commit_and_record_id", observed)
	if readErr != nil {
		return false
	}
	update := helperIPCSubmit(ctx, w, "reminders.update", map[string]any{"id": record.ID, "title": title + " requested"}, record.Version)
	prepare, err = helperIPCFrame(conn, wire, "Prepare")
	add("causal_update_prepared", err == nil)
	if err != nil {
		return false
	}
	second := contracts.Capture{Source: contracts.Reminders, Generation: checkpoint.Generation, Predecessor: first.Checkpoint, Checkpoint: contracts.NewID("cp"), Observations: []contracts.Observation{observation(title + " external")}}
	releaseConflict, err := helperIPCHold(w, wire, second)
	add("conflict_capture_held_before_receipt", err == nil)
	if err != nil {
		return false
	}
	defer releaseConflict()
	evidence, _ = json.Marshal(contracts.ConflictObservation{NativeKey: native, CaptureCheckpoint: second.Checkpoint})
	_, err = wire.Send(contracts.IPCFrame{T: "Rejected", CommandID: prepare.CommandID, Disposition: contracts.DispositionRejectedBeforeDispatch, Error: contracts.ErrConflict, Evidence: evidence})
	if err == nil {
		_, err = wire.Send(contracts.IPCFrame{T: "Ping"})
	}
	if err == nil {
		err = helperPong(conn, wire, time.Now().Add(time.Second))
	}
	if err != nil {
		add("conflict_control_progresses_before_receipt", false)
		return false
	}
	early = helperIPCEarly(update)
	add("conflict_reply_waits_for_capture_receipt", early == nil)
	if err = releaseConflict(); err != nil {
		return false
	}
	_, err = helperIPCFrame(conn, wire, "Committed")
	if err != nil {
		return false
	}
	result = helperIPCResult(update, early)
	var problem *contracts.Error
	if !errors.As(result.err, &problem) {
		problem = result.record.Error
	}
	code := ""
	if problem != nil {
		code = problem.Code
	}
	e.Metrics["ipc_conflict_response"] = map[string]any{"code": code, "has_current": problem != nil && problem.Current != nil}
	conflict := problem != nil && problem.Code == "conflict" && problem.Current != nil && problem.Current.ID == record.ID && problem.Current.Version > record.Version
	if conflict {
		var data struct {
			Title string `json:"title"`
		}
		conflict = json.Unmarshal(problem.Current.Data, &data) == nil && data.Title == title+" external"
	}
	add("cited_capture_returns_conflict_with_current", conflict)
	return true
}

func helperIPCSubmit(ctx context.Context, w *world.World, operation string, args map[string]any, version uint64) <-chan helperCommandResult {
	done := make(chan helperCommandResult, 1)
	go func() {
		bounded, cancel := context.WithTimeout(ctx, 5*time.Second)
		defer cancel()
		result := helperCommandResult{}
		client := scripted.Client{Endpoint: w.Gateway, Protocol: "rest"}
		result.err = client.Call(bounded, operation, map[string]any{"command_id": contracts.NewCommandID(), "args": args, "expected_version": version, "wait_ms": 4000}, &result.record)
		done <- result
	}()
	return done
}
func helperIPCEarly(done <-chan helperCommandResult) *helperCommandResult {
	select {
	case result := <-done:
		return &result
	case <-time.After(500 * time.Millisecond):
		return nil
	}
}
func helperIPCResult(done <-chan helperCommandResult, early *helperCommandResult) helperCommandResult {
	if early != nil {
		return *early
	}
	select {
	case result := <-done:
		return result
	case <-time.After(4 * time.Second):
		return helperCommandResult{err: contracts.ErrUnavailable}
	}
}
func helperIPCFrame(conn *net.UnixConn, wire *applepeer.Wire, kind string) (contracts.IPCFrame, error) {
	conn.SetReadDeadline(time.Now().Add(3 * time.Second))
	for {
		var frame contracts.IPCFrame
		if _, err := contracts.ReadFrame(conn, &frame); err != nil {
			return frame, err
		}
		if frame.T == kind {
			return frame, nil
		}
		if frame.T != "Ping" {
			return frame, fmt.Errorf("expected %s, received %s", kind, frame.T)
		}
		if _, err := wire.Send(contracts.IPCFrame{T: "Pong"}); err != nil {
			return frame, err
		}
	}
}
func helperIPCRecord(ctx context.Context, w *world.World, collection, title string) (contracts.Record, error) {
	var result contracts.ReadResult
	_, err := world.Request(ctx, w.Gateway, "/v1/op/records.query", contracts.ReadQuery{Collection: collection, Limit: 1000}, &result)
	if err != nil {
		return contracts.Record{}, err
	}
	for _, record := range result.Records {
		var data struct {
			Title string `json:"title"`
		}
		if json.Unmarshal(record.Data, &data) == nil && data.Title == title {
			return record, nil
		}
	}
	return contracts.Record{}, contracts.ErrNotFound
}
func helperIPCHold(w *world.World, wire *applepeer.Wire, capture contracts.Capture) (func() error, error) {
	gate := filepath.Join(w.Gateway.Home, "lab-command-capture-release")
	os.Remove(gate)
	os.Remove(filepath.Join(w.Gateway.Home, "lab-command-capture-reached"))
	if err := syscall.Mkfifo(gate, 0600); err != nil {
		return nil, err
	}
	if err := os.WriteFile(filepath.Join(w.Gateway.Home, "lab-command-capture-armed"), []byte("armed"), 0600); err != nil {
		return nil, err
	}
	if _, err := wire.Send(contracts.IPCFrame{T: "Capture", Capture: &capture}); err != nil {
		return nil, err
	}
	if err := w.Await(3*time.Second, func() bool {
		_, err := os.Stat(filepath.Join(w.Gateway.Home, "lab-command-capture-reached"))
		return err == nil
	}); err != nil {
		return nil, err
	}
	released := false
	return func() error {
		if released {
			return nil
		}
		released = true
		file, err := os.OpenFile(gate, os.O_WRONLY, 0)
		if err != nil {
			return err
		}
		defer file.Close()
		_, err = file.Write([]byte{1})
		return err
	}, nil
}
