package simapple

import (
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/applepeer"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"strings"
	"time"
)

type musicPlayer struct {
	State  string
	Volume int
	ID     string
	Step   int
}

func (m *musicPlayer) player() map[string]any {
	state := m.State
	if state == "" {
		state = "stopped"
	}
	return map[string]any{"state": state, "volume": m.Volume, "id": m.ID, "position": 0, "track": "Fixture", "artist": "Fixture", "album": "Fixture"}
}
func (f *fakeApple) musicCommand(wire *applepeer.Wire, frame contracts.IPCFrame, fence uint64, _ func(contracts.Capture, uint64) error, _ func(contracts.Source) string) (bool, error) {
	f.mu.Lock()
	defer f.mu.Unlock()
	if frame.T == "Prepare" {
		if frame.Action == nil || !strings.HasPrefix(frame.Action.Op, "music.") {
			return false, nil
		}
		id := contracts.NewID("ticket")
		f.tickets[id] = ticket{CommandID: frame.CommandID, Action: *frame.Action, Hash: frame.RequestHash, Fence: fence, Expires: time.Now().Add(30 * time.Second)}
		_, err := wire.Send(contracts.IPCFrame{T: "Prepared", CommandID: frame.CommandID, Ticket: id, Fingerprint: frame.RequestHash})
		return true, err
	}
	if frame.T != "Dispatch" {
		return false, nil
	}
	t, ok := f.tickets[frame.Ticket]
	if !ok || !strings.HasPrefix(t.Action.Op, "music.") {
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
	var args contracts.MusicCommandArgs
	json.Unmarshal(raw, &args)
	switch t.Action.Op {
	case "music.play":
		f.music.State = "playing"
	case "music.pause":
		f.music.State = "paused"
	case "music.play_item":
		f.music.ID = args.ID
		f.music.State = "playing"
	case "music.set_volume":
		if args.Volume == nil {
			return true, contracts.ErrInvalid
		}
		f.music.Volume = *args.Volume
	case "music.next", "music.previous":
		f.music.Step++
		if f.music.Step%2 == 0 {
			f.music.ID = "ABCDEF0123456789"
		} else {
			f.music.ID = "ABCDEF0123456780"
		}
	}
	evidence, _ := json.Marshal(map[string]any{"verified": true, "fingerprint": t.Hash, "player": f.music.player()})
	outcome := contracts.IPCFrame{T: "Outcome", CommandID: frame.CommandID, Disposition: "locally_observed", Evidence: evidence}
	if err := f.guard(guardEntry{CommandID: frame.CommandID, Ticket: frame.Ticket, Outcome: &outcome}); err != nil {
		return true, err
	}
	_, err := wire.Send(outcome)
	return true, err
}
func init() { sourceCommands = append(sourceCommands, (*fakeApple).musicCommand) }
