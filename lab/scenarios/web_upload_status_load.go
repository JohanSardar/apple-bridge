package scenarios

import (
	"bytes"
	"context"
	"os"
	"path/filepath"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"github.com/coder/websocket"
)

func uploadStatusSubmit(ctx context.Context, w *world.World, socket *websocket.Conn, command contracts.Command, lose bool, fault string, reverse bool, e *Evidence, add func(string, bool)) (contracts.CommandRecord, error) {
	if fault != "unrelated" && fault != "epoch" && fault != "capture" {
		return uploadSubmit(ctx, w, socket, command, lose)
	}
	home := w.Replicas[0].Home
	hold := filepath.Join(home, "command-visibility-hold")
	add("terminal_visibility_hold_armed", os.WriteFile(hold, []byte(command.ID), 0600) == nil)
	defer os.Remove(hold)
	add("status_lookup_counter_armed", os.WriteFile(filepath.Join(home, "command-status-probe"), []byte(command.ID), 0600) == nil)
	type result struct {
		record contracts.CommandRecord
		err    error
	}
	done := make(chan result, 1)
	go func() { record, err := uploadCommandReply(ctx, socket, command); done <- result{record, err} }()
	err := w.Await(10*time.Second, func() bool {
		raw, _ := os.ReadFile(filepath.Join(home, "command-status-held"))
		return string(raw) == command.ID
	})
	add("committed_status_held_before_visibility", err == nil)
	if err == nil {
		if fault == "unrelated" {
			uploadStatusUnrelated(ctx, w, command.ID, reverse, e, add)
		} else if fault == "capture" {
			uploadStatusCapture(w, command.ID, add)
		} else {
			uploadStatusEpoch(w, add)
		}
	}
	os.Remove(hold)
	if fault == "unrelated" || fault == "capture" {
		uploadStatusChange(w, contracts.NewID("status-release"))
	}
	select {
	case reply := <-done:
		return reply.record, reply.err
	case <-ctx.Done():
		return contracts.CommandRecord{}, ctx.Err()
	}
}
func uploadStatusChange(w *world.World, key string) error {
	if err := w.ControlAction("/external-change", map[string]any{"key": key, "collection": "messages.message", "data": map[string]any{"title": "[ab-test] " + key, "text": "Bridge Test unrelated change"}}); err != nil {
		return err
	}
	if err := w.ControlAction("/flush", struct{}{}); err != nil {
		return err
	}
	head, err := w.Cursor(w.Gateway)
	if err != nil {
		return err
	}
	return w.Await(5*time.Second, func() bool {
		local, err := w.Cursor(w.Replicas[0])
		return err == nil && local.Epoch == head.Epoch && local.Sequence >= head.Sequence
	})
}
func uploadStatusUnrelated(ctx context.Context, w *world.World, id string, reverse bool, e *Evidence, add func(string, bool)) {
	count := 0
	for ; count < 100; count++ {
		if uploadStatusChange(w, contracts.NewID("status-unrelated")) != nil {
			break
		}
	}
	add("hundred_unrelated_changes_replicated", count == 100)
	raw, _ := os.ReadFile(filepath.Join(w.Replicas[0].Home, "command-status-lookups"))
	calls := bytes.Count(raw, []byte("\n"))
	record, err := uploadGatewayCommand(ctx, w.Gateway.Home, id)
	direction := "replica_dials"
	if reverse {
		direction = "gateway_dials"
	}
	e.Metrics[direction+"_unrelated_status_calls"] = calls
	e.Metrics[direction+"_command_status_transitions"] = len(record.History)
	e.Metrics[direction+"_unrelated_changes"] = count
	add("remote_status_calls_bounded_by_transitions_plus_one", err == nil && calls <= len(record.History)+1)
}
func uploadStatusEpoch(w *world.World, add func(string, bool)) {
	before, err := w.Cursor(w.Gateway)
	backup := filepath.Join(w.Dir, "command-visible.db")
	add("committed_snapshot_backup", err == nil && w.CommandBackup(backup) == nil)
	w.StopGateway()
	add("gateway_epoch_reset", w.CommandRestore(backup, false) == nil)
	add("gateway_restart_after_epoch_reset", w.RestartGateway() == nil)
	err = w.Await(10*time.Second, func() bool {
		head, err := w.Cursor(w.Replicas[0])
		status, statusErr := w.Status(w.Replicas[0])
		return err == nil && statusErr == nil && head.Epoch > before.Epoch && status.Snapshots >= 2
	})
	add("newer_epoch_snapshot_installed_before_status_visibility", err == nil)
}
