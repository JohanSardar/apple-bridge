package simapple

import (
	"encoding/json"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/applepeer"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func (f *fakeApple) safariCommand(wire *applepeer.Wire, frame contracts.IPCFrame, fence uint64, emit func(contracts.Capture, uint64) error, checkpoint func(contracts.Source) string) (bool, error) {
	f.mu.Lock()
	defer f.mu.Unlock()
	if frame.T == "Prepare" {
		if frame.Action == nil || !strings.HasPrefix(frame.Action.Op, "safari.") {
			return false, nil
		}
		if frame.Action.Op == "safari.close_tab" {
			_, err := wire.Send(contracts.IPCFrame{T: "Rejected", CommandID: frame.CommandID, Disposition: "rejected_before_dispatch", Error: contracts.ErrInvalid})
			return true, err
		}
		id := contracts.NewID("ticket")
		f.tickets[id] = ticket{CommandID: frame.CommandID, Action: *frame.Action, Hash: frame.RequestHash, Fence: fence, Expires: time.Now().Add(30 * time.Second)}
		_, err := wire.Send(contracts.IPCFrame{T: "Prepared", CommandID: frame.CommandID, Ticket: id, Route: "safari_readback", Fingerprint: frame.RequestHash})
		return true, err
	}
	if frame.T != "Dispatch" {
		return false, nil
	}
	t, ok := f.tickets[frame.Ticket]
	if !ok || !strings.HasPrefix(t.Action.Op, "safari.") {
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
	var args contracts.SafariArgs
	if json.Unmarshal(raw, &args) != nil {
		return true, contracts.ErrInvalid
	}
	if f.safariOwned == nil {
		f.safariOwned = map[string]string{}
	}
	var readback any
	var o contracts.Observation
	switch t.Action.Op {
	case "safari.open_url":
		key := contracts.NewID("tab")
		f.safariOwned[key] = args.URL
		readback = map[string]any{"tab": key, "url": args.URL}
		data, _ := json.Marshal(map[string]any{"window": 1, "index": 1, "title": "Synthetic", "url": args.URL, "active": true})
		o = contracts.Observation{Kind: "upsert", Collection: "safari.tab", NativeKey: key, Data: data}
	case "safari.add_reading_list":
		readback = map[string]any{"url": args.URL}
		data, _ := json.Marshal(map[string]any{"url": args.URL, "title": "Synthetic", "added": nil, "read": false})
		o = contracts.Observation{Kind: "upsert", Collection: "safari.reading_list", NativeKey: contracts.NewID("reading"), Data: data}
	default:
		return true, contracts.ErrInvalid
	}

	o.ObservedAt = time.Now().UTC()
	o.ScopedAlias = "safari/" + o.Collection + "/" + o.NativeKey
	if o.Kind == "absent" {
		delete(f.records, o.Collection+"/"+o.NativeKey)
	} else {
		f.records[o.Collection+"/"+o.NativeKey] = o
	}
	persistErr := f.changedLocked(contracts.Safari)
	revision := f.durable.Revision[contracts.Safari]
	if persistErr != nil {
		return true, persistErr
	}
	f.mu.Unlock()
	emitErr := emit(fullScanSource([]contracts.Observation{o}, contracts.Safari), revision)
	f.mu.Lock()
	if emitErr != nil {
		return true, emitErr
	}
	evidence, _ := json.Marshal(map[string]any{"observed": true, "fingerprint": t.Hash, "readback": readback})
	outcome := contracts.IPCFrame{T: "Outcome", CommandID: frame.CommandID, Disposition: "locally_observed", Evidence: evidence}
	if err := f.guard(guardEntry{CommandID: frame.CommandID, Ticket: frame.Ticket, Outcome: &outcome}); err != nil {
		return true, err
	}
	_, err := wire.Send(outcome)
	return true, err
}

func init() { sourceCommands = append(sourceCommands, (*fakeApple).safariCommand) }
