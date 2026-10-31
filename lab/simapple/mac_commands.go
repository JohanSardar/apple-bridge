package simapple

import (
	"encoding/json"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/applepeer"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func (f *fakeApple) macCommand(wire *applepeer.Wire, frame contracts.IPCFrame, fence uint64, emit func(contracts.Capture, uint64) error, checkpoint func(contracts.Source) string) (bool, error) {
	f.mu.Lock()
	defer f.mu.Unlock()
	if frame.T == "Prepare" {
		if frame.Action == nil || !strings.HasPrefix(frame.Action.Op, "mac.") {
			return false, nil
		}
		if frame.Action.Op == "mac.notify" && f.notificationPermission != "" && f.notificationPermission != "authorized" {
			_, err := wire.Send(contracts.IPCFrame{T: "Rejected", CommandID: frame.CommandID, Disposition: "rejected_before_dispatch", Error: contracts.ErrForbidden})
			return true, err
		}
		id := contracts.NewID("ticket")
		f.tickets[id] = ticket{CommandID: frame.CommandID, Action: *frame.Action, Hash: frame.RequestHash, Fence: fence, Expires: time.Now().Add(30 * time.Second)}
		_, err := wire.Send(contracts.IPCFrame{T: "Prepared", CommandID: frame.CommandID, Ticket: id, Route: "mac_readback", Fingerprint: frame.RequestHash})
		return true, err
	}
	if frame.T != "Dispatch" {
		return false, nil
	}
	t, ok := f.tickets[frame.Ticket]
	if !ok || !strings.HasPrefix(t.Action.Op, "mac.") {
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
	var args contracts.MacArgs
	if json.Unmarshal(raw, &args) != nil {
		return true, contracts.ErrInvalid
	}
	var readback any
	switch t.Action.Op {
	case "mac.iwork_export":
		readback = map[string]any{"app": args.App, "format": args.Format, "data_base64": fakeIWorkExport(args.Format)}
	case "mac.notify":
		readback = map[string]any{"delivered": true}
	case "mac.open_settings":
		readback = map[string]any{"pane": args.Pane, "frontmost": true}
	case "mac.set_wallpaper":
		readback = map[string]any{"matches": true}
	case "mac.lock_screen", "mac.sleep_display", "mac.restart", "mac.shutdown":
		readback = map[string]any{"dispatched": true}
	case "mac.clipboard_write":
		if args.Text == nil {
			return true, contracts.ErrInvalid
		}
		f.clipboard = *args.Text
		readback = true
	case "mac.set_volume":
		readback = args.Volume
	case "mac.set_mute":
		readback = args.Muted
	case "mac.set_output_device":
		readback = args.Device
	case "mac.set_appearance":
		readback = args.Appearance
	case "mac.keep_awake":
		readback = args.On
	default:
		readback = map[string]any{"bundle_id": args.BundleID, "running": t.Action.Op != "mac.quit_app"}
	}
	evidence, _ := json.Marshal(map[string]any{"observed": true, "fingerprint": t.Hash, "readback": readback})
	outcome := contracts.IPCFrame{T: "Outcome", CommandID: frame.CommandID, Disposition: "locally_observed", Evidence: evidence}
	if err := f.guard(guardEntry{CommandID: frame.CommandID, Ticket: frame.Ticket, Outcome: &outcome}); err != nil {
		return true, err
	}
	_, err := wire.Send(outcome)
	return true, err
}

func init() { sourceCommands = append(sourceCommands, (*fakeApple).macCommand) }
