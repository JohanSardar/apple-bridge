package simapple

import (
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/applepeer"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"time"
)

func (f *fakeApple) phoneCommand(wire *applepeer.Wire, frame contracts.IPCFrame, fence uint64, emit func(contracts.Capture, uint64) error, checkpoint func(contracts.Source) string) (bool, error) {
	f.mu.Lock()
	if frame.T == "Prepare" && frame.Action != nil && frame.Action.Op == "phone.dial" {
		id := contracts.NewID("ticket")
		f.tickets[id] = ticket{CommandID: frame.CommandID, Action: *frame.Action, Hash: frame.RequestHash, Fence: fence, Expires: time.Now().Add(30 * time.Second)}
		f.mu.Unlock()
		_, err := wire.Send(contracts.IPCFrame{T: "Prepared", CommandID: frame.CommandID, Ticket: id, Route: "fake_continuity", Fingerprint: frame.RequestHash})
		return true, err
	}
	t, ok := f.tickets[frame.Ticket]
	if frame.T != "Dispatch" || !ok || t.Action.Op != "phone.dial" {
		f.mu.Unlock()
		return false, nil
	}
	delete(f.tickets, frame.Ticket)
	fault := f.fault
	f.mu.Unlock()
	if t.CommandID != frame.CommandID || t.Fence != fence || !time.Now().Before(t.Expires) {
		_, err := wire.Send(contracts.IPCFrame{T: "Rejected", CommandID: frame.CommandID, Disposition: "rejected_before_dispatch", Error: contracts.ErrExpired})
		return true, err
	}
	if err := f.guard(guardEntry{CommandID: frame.CommandID, Ticket: frame.Ticket, StartedAt: time.Now()}); err != nil {
		return true, err
	}
	outcome := contracts.IPCFrame{T: "Outcome", CommandID: frame.CommandID, Disposition: "indeterminate"}
	if fault != "dial-no-record" {
		raw, _ := json.Marshal(t.Action.Args)
		var args contracts.PhoneDialArgs
		if json.Unmarshal(raw, &args) != nil {
			return true, contracts.ErrInvalid
		}
		now := time.Now().UTC()
		data, _ := json.Marshal(map[string]any{"direction": "outgoing", "handle": args.Number, "name": "Fixture", "date": now, "duration": 0, "service": "cellular", "answered": false})
		key := contracts.NewID("call")
		observation := contracts.Observation{Kind: "upsert", Collection: "phone.call", NativeKey: key, ScopedAlias: "phone/fixture/" + key, Data: data, ObservedAt: now}
		f.mu.Lock()
		f.records[key] = observation
		err := f.changedLocked(contracts.Phone)
		revision := f.durable.Revision[contracts.Phone]
		f.mu.Unlock()
		if err != nil {
			return true, err
		}
		if err = emit(contracts.Capture{Source: contracts.Phone, Observations: []contracts.Observation{observation}}, revision); err != nil {
			return true, err
		}
		outcome.Disposition = "locally_observed"
		outcome.Evidence, _ = json.Marshal(map[string]any{"native_key": key, "observed": true, "fingerprint": t.Hash, "capture_checkpoint": checkpoint(contracts.Phone)})
	}
	if err := f.guard(guardEntry{CommandID: frame.CommandID, Ticket: frame.Ticket, Outcome: &outcome}); err != nil {
		return true, err
	}
	_, err := wire.Send(outcome)
	return true, err
}

func init() { sourceCommands = append(sourceCommands, (*fakeApple).phoneCommand) }
