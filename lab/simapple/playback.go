package simapple

import (
	"encoding/json"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/applepeer"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func (f *fakeApple) playbackCommand(wire *applepeer.Wire, frame contracts.IPCFrame, fence uint64, emit func(contracts.Capture, uint64) error, checkpoint func(contracts.Source) string) (bool, error) {
	f.mu.Lock()
	defer f.mu.Unlock()
	if frame.T == "Prepare" {
		if frame.Action == nil || !contracts.PlaybackOperation(frame.Action.Op) {
			return false, nil
		}
		id := contracts.NewID("ticket")
		f.tickets[id] = ticket{CommandID: frame.CommandID, Action: *frame.Action, Hash: frame.RequestHash, Fence: fence, Expires: time.Now().Add(30 * time.Second)}
		_, err := wire.Send(contracts.IPCFrame{T: "Prepared", CommandID: frame.CommandID, Ticket: id, Route: "playback", Fingerprint: frame.RequestHash})
		return true, err
	}
	if frame.T != "Dispatch" {
		return false, nil
	}
	t, ok := f.tickets[frame.Ticket]
	if !ok || !contracts.PlaybackOperation(t.Action.Op) {
		return false, nil
	}
	delete(f.tickets, frame.Ticket)
	if t.CommandID != frame.CommandID || t.Fence != fence || !time.Now().Before(t.Expires) {
		_, err := wire.Send(contracts.IPCFrame{T: "Rejected", CommandID: frame.CommandID, Disposition: "rejected_before_dispatch", Error: contracts.ErrExpired})
		return true, err
	}
	if err := f.guard(guardEntry{CommandID: frame.CommandID, Ticket: frame.Ticket, StartedAt: time.Now()}); err != nil {
		return true, err
	}
	raw, _ := json.Marshal(t.Action.Args)
	var args contracts.PlaybackArgs
	if json.Unmarshal(raw, &args) != nil {
		return true, contracts.ErrInvalid
	}
	finished := make(chan struct{}, 1)
	if args.Text != "[ab-test missing callback]" {
		finished <- struct{}{}
	}
	callback := false
	timer := time.NewTimer(50 * time.Millisecond)
	defer timer.Stop()
	select {
	case <-finished:
		callback = true
	case <-timer.C:
	}
	disposition := contracts.DispositionIndeterminate
	if callback {
		disposition = contracts.DispositionLocallyObserved
	}
	evidence, _ := json.Marshal(map[string]any{"finished_callback": callback, "observed": callback, "prepared": map[string]any{"request_hash": t.Hash}})
	outcome := contracts.IPCFrame{T: "Outcome", CommandID: frame.CommandID, Disposition: disposition, Evidence: evidence}
	if err := f.guard(guardEntry{CommandID: frame.CommandID, Ticket: frame.Ticket, Outcome: &outcome}); err != nil {
		return true, err
	}
	_, err := wire.Send(outcome)
	return true, err
}

func init() { sourceCommands = append(sourceCommands, (*fakeApple).playbackCommand) }
