package simapple

import (
	"bytes"
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/applepeer"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"golang.org/x/net/html"
	"io"
	"reflect"
	"strings"
	"time"
)

func (f *fakeApple) noteCommand(wire *applepeer.Wire, frame contracts.IPCFrame, fence uint64, emit func(contracts.Capture, uint64) error, checkpoint func() string) error {
	reject := func(problem *contracts.Error) error {
		_, err := wire.Send(contracts.IPCFrame{T: "Rejected", CommandID: frame.CommandID, Disposition: "rejected_before_dispatch", Error: problem})
		return err
	}
	switch frame.T {
	case "Prepare":
		if frame.Action == nil || frame.Action.NoteArgs == nil {
			return reject(contracts.ErrInvalid)
		}
		f.mu.Lock()
		action := *frame.Action
		var before json.RawMessage
		if action.NativeKey != "" {
			o, ok := f.records[action.NativeKey]
			if !ok || o.Kind == "absent" {
				f.mu.Unlock()
				return reject(contracts.ErrConflict)
			}
			before = o.Data
			var expected, native map[string]any
			json.Unmarshal(frame.ExpectedState, &expected)
			json.Unmarshal(before, &native)
			delete(expected, "content_version")
			delete(expected, "folder_id")
			delete(native, "folder_id")
			if !reflect.DeepEqual(expected, native) {
				f.mu.Unlock()
				return reject(contracts.ErrConflict)
			}

			var data map[string]any
			json.Unmarshal(before, &data)
			if action.Op == "notes.update" && (action.NoteArgs.HTML == nil && data["editable"] != "plain" || action.NoteArgs.HTML != nil && !contracts.NoteHTMLCompatible(before, *action.NoteArgs.HTML)) {
				f.mu.Unlock()
				return reject(contracts.ErrUnsupported)
			}
		}
		ticketID := contracts.NewID("ticket")
		f.tickets[ticketID] = ticket{CommandID: frame.CommandID, Principal: frame.Principal, Hash: frame.RequestHash, Action: action, Before: before, Expires: time.Now().Add(30 * time.Second), Fence: fence}
		f.mu.Unlock()
		_, err := wire.Send(contracts.IPCFrame{T: "Prepared", CommandID: frame.CommandID, Ticket: ticketID, NativeState: before, Route: "applescript", Fingerprint: frame.RequestHash})
		return err
	case "Dispatch":
		f.mu.Lock()
		t, ok := f.tickets[frame.Ticket]
		delete(f.tickets, frame.Ticket)
		if !ok || t.CommandID != frame.CommandID || t.Fence != fence || !time.Now().Before(t.Expires) {
			f.mu.Unlock()
			return reject(contracts.ErrExpired)
		}
		if t.Action.NativeKey != "" && !bytes.Equal(f.records[t.Action.NativeKey].Data, t.Before) {
			f.mu.Unlock()
			return reject(contracts.ErrConflict)
		}
		f.mu.Unlock()
		if err := f.guard(guardEntry{CommandID: frame.CommandID, Ticket: frame.Ticket, StartedAt: time.Now()}); err != nil {
			return err
		}
		f.mu.Lock()
		a := t.Action.NoteArgs
		key := t.Action.NativeKey
		collection := "notes.note"
		data := map[string]any{}
		if key != "" {
			json.Unmarshal(f.records[key].Data, &data)
		} else {
			key = contracts.NewID("note")
		}
		if t.Action.Op == "notes.rename_folder" {
			collection = "notes.folder"
			data["title"] = *a.Title
		} else if t.Action.Op == "notes.create_folder" {
			collection = "notes.folder"
			data = map[string]any{"title": *a.Title, "account": "sim"}
		} else if t.Action.Op == "notes.create" {
			text := ""
			if a.PlainText != nil {
				text = *a.PlainText
			}
			data = map[string]any{"folder_id": map[string]any{"$ref": map[string]string{"collection": "notes.folder", "native_key": t.Action.FolderKey}}, "title": *a.Title, "plain_text": *a.Title + "\n" + text, "locked": false, "editable": "plain", "content_status": "complete", "attachments": []any{}, "created_at": time.Now().UTC().Format(time.RFC3339Nano), "modified_at": time.Now().UTC().Format(time.RFC3339Nano)}
		} else if t.Action.Op == "notes.move" {
			data["folder_id"] = map[string]any{"$ref": map[string]string{"collection": "notes.folder", "native_key": t.Action.FolderKey}}
		} else if t.Action.Op == "notes.update" {
			text := ""
			if a.HTML != nil {
				data["html"] = *a.HTML
				tokenizer := html.NewTokenizer(strings.NewReader(*a.HTML))
				for {
					token := tokenizer.Next()
					if token == html.ErrorToken {
						if tokenizer.Err() != io.EOF {
							return tokenizer.Err()
						}
						break
					}
					if token == html.TextToken {
						text += string(tokenizer.Text())
					}
					if token == html.EndTagToken {
						name, _ := tokenizer.TagName()
						if string(name) == "div" {
							text += "\n"
						}
					}
				}
			} else {
				text = *a.PlainText
			}
			if a.Mode == "append" {
				text = data["plain_text"].(string) + text
			}
			data["plain_text"] = text
			data["title"] = strings.SplitN(text, "\n", 2)[0]
			data["modified_at"] = time.Now().UTC().Format(time.RFC3339Nano)
		}
		raw, _ := json.Marshal(data)
		o := contracts.Observation{Kind: "upsert", Collection: collection, NativeKey: key, Data: raw, ObservedAt: time.Now().UTC()}
		if t.Action.Op == "notes.delete" {
			o.Kind = "absent"
			o.Revalidated = true
		}
		f.records[key] = o
		f.effects[frame.CommandID]++
		err := f.changedLocked(contracts.Notes)
		revision := f.durable.Revision[contracts.Notes]
		f.mu.Unlock()
		if err != nil {
			return err
		}
		if err = emit(fullScanSource([]contracts.Observation{o}, contracts.Notes), revision); err != nil {
			return err
		}
		evidence, _ := json.Marshal(map[string]any{"native_key": key, "observed": true, "absent": o.Kind == "absent", "fingerprint": t.Hash, "capture_checkpoint": checkpoint()})
		outcome := contracts.IPCFrame{T: "Outcome", CommandID: frame.CommandID, Disposition: "locally_observed", Evidence: evidence}
		f.mu.Lock()
		f.evidence[frame.CommandID] = outcome
		err = f.saveLocked()
		f.mu.Unlock()
		if err != nil {
			return err
		}
		if err = f.guard(guardEntry{CommandID: frame.CommandID, Outcome: &outcome}); err != nil {
			return err
		}
		_, err = wire.Send(outcome)
		return err
	}
	return reject(contracts.ErrUnsupported)
}

func (f *fakeApple) routeNotesCommand(wire *applepeer.Wire, frame contracts.IPCFrame, fence uint64, emit func(contracts.Capture, uint64) error, checkpoint func(contracts.Source) string) (bool, error) {
	if frame.Action != nil && strings.HasPrefix(frame.Action.Op, "notes.") {
		return true, f.noteCommand(wire, frame, fence, emit, func() string { return checkpoint(contracts.Notes) })
	}
	if frame.T == "Dispatch" {
		f.mu.Lock()
		t, ok := f.tickets[frame.Ticket]
		f.mu.Unlock()
		if ok && strings.HasPrefix(t.Action.Op, "notes.") {
			return true, f.noteCommand(wire, frame, fence, emit, func() string { return checkpoint(contracts.Notes) })
		}
	}
	return false, nil
}

func init() { sourceCommands = append(sourceCommands, (*fakeApple).routeNotesCommand) }
