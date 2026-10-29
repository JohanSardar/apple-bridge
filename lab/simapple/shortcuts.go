package simapple

import (
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/applepeer"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func (f *fakeApple) shortcutCommand(wire *applepeer.Wire, frame contracts.IPCFrame, fence uint64, emit func(contracts.Capture, uint64) error, checkpoint func(contracts.Source) string) (bool, error) {
	f.mu.Lock()
	defer f.mu.Unlock()
	if frame.T == "Prepare" {
		if frame.Action == nil || frame.Action.Op != "shortcuts.run" {
			return false, nil
		}
		id := contracts.NewID("ticket")
		f.tickets[id] = ticket{CommandID: frame.CommandID, Action: *frame.Action, Hash: frame.RequestHash, Fence: fence, Expires: time.Now().Add(30 * time.Second)}
		_, err := wire.Send(contracts.IPCFrame{T: "Prepared", CommandID: frame.CommandID, Ticket: id, Route: "shortcuts_cli", Fingerprint: frame.RequestHash})
		return true, err
	}
	if frame.T != "Dispatch" {
		return false, nil
	}
	t, ok := f.tickets[frame.Ticket]
	if !ok || t.Action.Op != "shortcuts.run" {
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
	var args contracts.ShortcutRunArgs
	if json.Unmarshal(raw, &args) != nil {
		return true, contracts.ErrInvalid
	}
	if args.Shortcut == "[ab-test slow runner]" {
		for i := 0; i < 4; i++ {
			time.Sleep(4 * time.Second)
			if _, err := wire.Send(contracts.IPCFrame{T: "Pong"}); err != nil {
				return true, err
			}
		}
	}
	data := []byte(args.Input)
	if args.InputBase64 != "" {
		data, _ = base64.StdEncoding.DecodeString(args.InputBase64)
	}
	digest := sha256.Sum256(data)
	evidence, _ := json.Marshal(map[string]any{"output_sha256": hex.EncodeToString(digest[:]), "output_bytes": len(data), "exit_status": 0, "output_base64": base64.StdEncoding.EncodeToString(data), "effect_scope": "runner completion only", "prepared": map[string]any{"request_hash": t.Hash}})
	outcome := contracts.IPCFrame{T: "Outcome", CommandID: frame.CommandID, Disposition: "locally_observed", Evidence: evidence}
	if err := f.guard(guardEntry{CommandID: frame.CommandID, Ticket: frame.Ticket, Outcome: &outcome}); err != nil {
		return true, err
	}
	_, err := wire.Send(outcome)
	return true, err
}

func init() { sourceCommands = append(sourceCommands, (*fakeApple).shortcutCommand) }
