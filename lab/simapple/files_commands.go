package simapple

import (
	"encoding/base64"
	"encoding/json"
	"path"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/applepeer"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func (f *fakeApple) fileCommand(wire *applepeer.Wire, frame contracts.IPCFrame, fence uint64, emit func(contracts.Capture, uint64) error, checkpoint func(contracts.Source) string) (bool, error) {
	f.mu.Lock()
	if frame.T == "Prepare" {
		if frame.Action == nil || !strings.HasPrefix(frame.Action.Op, "files.") {
			f.mu.Unlock()
			return false, nil
		}
		raw, _ := json.Marshal(frame.Action.Args)
		var a contracts.FileArgs
		problem := contracts.ValidateFileCommand(contracts.Command{ID: frame.CommandID, Op: frame.Action.Op, Args: raw})
		if problem == nil {
			problem = json.Unmarshal(raw, &a)
		}
		if problem != nil || a.Root != "/Bridge Test" {
			f.mu.Unlock()
			_, err := wire.Send(contracts.IPCFrame{T: "Rejected", CommandID: frame.CommandID, Error: contracts.ErrInvalid})
			return true, err
		}
		id := contracts.NewID("ticket")
		f.tickets[id] = ticket{CommandID: frame.CommandID, Action: *frame.Action, Hash: frame.RequestHash, Fence: fence, Expires: time.Now().Add(30 * time.Second)}
		f.mu.Unlock()
		_, err := wire.Send(contracts.IPCFrame{T: "Prepared", CommandID: frame.CommandID, Ticket: id, Fingerprint: frame.RequestHash})
		return true, err
	}
	if frame.T != "Dispatch" {
		f.mu.Unlock()
		return false, nil
	}
	t, ok := f.tickets[frame.Ticket]
	if !ok || !strings.HasPrefix(t.Action.Op, "files.") {
		f.mu.Unlock()
		return false, nil
	}
	delete(f.tickets, frame.Ticket)
	if t.CommandID != frame.CommandID || t.Fence != fence || !time.Now().Before(t.Expires) {
		f.mu.Unlock()
		_, err := wire.Send(contracts.IPCFrame{T: "Rejected", CommandID: frame.CommandID, Error: contracts.ErrExpired})
		return true, err
	}
	if err := f.guard(guardEntry{CommandID: frame.CommandID, Ticket: frame.Ticket, StartedAt: time.Now()}); err != nil {
		f.mu.Unlock()
		return true, err
	}
	raw, _ := json.Marshal(t.Action.Args)
	var a contracts.FileArgs
	json.Unmarshal(raw, &a)
	if a.Tags == nil {
		a.Tags = []string{}
	}
	op := t.Action.Op
	key := path.Join(a.Root, a.Path)
	target := path.Join(a.Root, a.Parent, a.Name)
	if op == "files.create" || op == "files.write" {
		key = target
	}
	row, exists := f.records[key]
	if exists && row.Kind == "absent" {
		exists = false
	}
	if op == "files.create" && exists || op != "files.create" && !exists {
		f.mu.Unlock()
		_, err := wire.Send(contracts.IPCFrame{T: "Outcome", CommandID: frame.CommandID, Disposition: "failed_no_effect", Error: contracts.ErrConflict})
		return true, err
	}
	data := map[string]any{}
	if exists {
		json.Unmarshal(row.Data, &data)
	}
	observations := []contracts.Observation{}
	evidence := map[string]any{"observed": true, "fingerprint": t.Hash, "path": key}
	if op == "files.create" || op == "files.write" {
		bytes, _ := base64.StdEncoding.DecodeString(a.Content)
		f.blobs[a.Digest] = bytes
		data = map[string]any{"root": a.Root, "path": strings.TrimPrefix(key, a.Root+"/"), "name": a.Name, "kind": "txt", "size": len(bytes), "created": nil, "modified": nil, "tags": []string{}, "downloaded": true, "icloud_status": "local", "digest": a.Digest}
		evidence["digest"] = a.Digest
	}
	if op == "files.move" || op == "files.rename" || op == "files.trash" {
		if op != "files.trash" {
			if other, ok := f.records[target]; ok && other.Kind != "absent" {
				f.mu.Unlock()
				_, err := wire.Send(contracts.IPCFrame{T: "Outcome", CommandID: frame.CommandID, Disposition: "failed_no_effect", Error: contracts.ErrConflict})
				return true, err
			}
		}
		absent := contracts.Observation{Kind: "absent", Collection: "files.item", NativeKey: key, ObservedAt: time.Now().UTC(), Revalidated: true}
		f.records[key] = absent
		observations = append(observations, absent)
		if op == "files.trash" {
			evidence["trashed"] = true
			evidence["trash_path"] = "/.Trash/" + frame.CommandID
		} else {
			key = target
			data["path"] = strings.TrimPrefix(key, a.Root+"/")
			data["name"] = a.Name
			evidence["path"] = key
		}
	}
	if op == "files.set_tags" {
		data["tags"] = a.Tags
		evidence["tags"] = a.Tags
	}
	if op == "files.download" || op == "files.evict" {
		data["downloaded"] = op == "files.download"
		evidence["downloaded"] = op == "files.download"
	}
	if op != "files.trash" {
		encoded, _ := json.Marshal(data)
		row = contracts.Observation{Kind: "upsert", Collection: "files.item", NativeKey: key, ObservedAt: time.Now().UTC(), Data: encoded}
		f.records[key] = row
		observations = append(observations, row)
	}
	err := f.changedLocked("files")
	revision := f.durable.Revision["files"]
	f.mu.Unlock()
	if err != nil {
		return true, err
	}
	if err = emit(contracts.Capture{Source: "files", Observations: observations, ScanID: contracts.NewID("scan"), Scope: []string{"files.item"}, ScanStartedAt: time.Now().UTC(), CompletionEvidence: "complete"}, revision); err != nil {
		return true, err
	}
	evidence["native_key"] = key
	evidence["capture_checkpoint"] = checkpoint("files")
	encoded, _ := json.Marshal(evidence)
	outcome := contracts.IPCFrame{T: "Outcome", CommandID: frame.CommandID, Disposition: "locally_observed", Evidence: encoded}
	f.mu.Lock()
	err = f.guard(guardEntry{CommandID: frame.CommandID, Ticket: frame.Ticket, Outcome: &outcome})
	f.mu.Unlock()
	if err != nil {
		return true, err
	}
	_, err = wire.Send(outcome)
	return true, err
}

func init() { sourceCommands = append(sourceCommands, (*fakeApple).fileCommand) }

func fileCapabilities() []string {
	var names []string
	for _, descriptor := range contracts.FileCommandDescriptors() {
		names = append(names, descriptor.Name)
	}
	return names
}
