package simapple

import (
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"github.com/JohanSardar/apple-bridge/internal/applepeer"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"os"
	"reflect"
	"strings"
	"time"
)

func messageCoverage() contracts.Coverage {
	c := contracts.Coverage{Fields: "complete", Missing: []string{}}
	for _, op := range contracts.MessagesOperations {
		c.Operations = append(c.Operations, contracts.OperationCapability{Operation: op, Proven: true, Reason: "simulated_native_route"})
	}
	return c
}
func messageCapabilities() []string { return contracts.MessagesOperations }
func (f *fakeApple) isMessageCommand(frame contracts.IPCFrame) bool {
	if frame.Action != nil {
		return strings.HasPrefix(frame.Action.Op, "messages.")
	}
	f.mu.Lock()
	defer f.mu.Unlock()
	if t, ok := f.tickets[frame.Ticket]; ok {
		return strings.HasPrefix(t.Action.Op, "messages.")
	}
	var e struct {
		Source string `json:"source"`
	}
	json.Unmarshal(f.evidence[frame.CommandID].Evidence, &e)
	return e.Source == "messages"
}
func messageProblem(code string) *contracts.Error { return &contracts.Error{Code: code, Message: code} }
func simGroupHandle(handle string) bool {
	return handle == "self@example.com" || handle == "second-self@example.com" || handle == "member1@example.com" || handle == "member2@example.com" || handle == "member3@example.com"
}
func simHandleKey(handle string) string {
	if handle == "self@example.com" {
		return "message-self"
	}
	if handle == "second-self@example.com" {
		return "message-self-email"
	}
	return "synthetic-" + strings.Split(handle, "@")[0]
}
func (f *fakeApple) messageValidate(a contracts.NativeAction) *contracts.Error {
	m := a.Messages
	if m == nil {
		return contracts.ErrInvalid
	}
	if m.ResendOf != "" {
		var previous struct {
			FromMe bool `json:"is_from_me"`
			Error  int  `json:"error_code"`
		}
		if json.Unmarshal(f.records[m.ResendOf].Data, &previous) != nil || !previous.FromMe || previous.Error == 0 {
			return contracts.ErrConflict
		}
	}
	if !f.messages.permitted || f.messages.capabilityUnproven {
		return messageProblem("capability_unproven")
	}
	for _, file := range m.Files {
		if file.Path == "" || file.Digest == "" {
			return contracts.ErrInvalid
		}
	}
	if a.Origin != "interactive" && m.Text != "" && (!strings.Contains(m.Text, "[ab-test ") || !strings.Contains(m.Text, "]")) {
		return contracts.ErrInvalid
	}
	if a.Origin != "interactive" && (a.Op == "messages.send" || a.Op == "messages.create_group") {
		for _, h := range m.Handles {
			if h != "self@example.com" && h != "second-self@example.com" && (a.Op != "messages.create_group" || !simGroupHandle(h)) {
				return contracts.ErrUnauthorized
			}
		}
	}
	if a.Origin != "interactive" && m.Handle != "" && !simGroupHandle(m.Handle) {
		return contracts.ErrUnauthorized
	}
	var data map[string]any
	if a.NativeKey != "" {
		o, ok := f.records[a.NativeKey]
		if !ok || o.Kind == "absent" {
			return contracts.ErrNotFound
		}
		json.Unmarshal(o.Data, &data)
	}
	chat := a.ListKey
	if data != nil {
		if o := f.records[a.NativeKey]; o.Collection == "messages.chat" {
			chat = a.NativeKey
		} else {
			raw, _ := json.Marshal(data["chat_id"])
			var ref struct {
				Ref struct {
					NativeKey string `json:"native_key"`
				} `json:"$ref"`
			}
			json.Unmarshal(raw, &ref)
			chat = ref.Ref.NativeKey
		}
	}
	service := m.Service
	if chat != "" {
		o, ok := f.records[chat]
		if !ok || o.Kind == "absent" {
			return contracts.ErrNotFound
		}
		var c map[string]any
		json.Unmarshal(o.Data, &c)
		service, _ = c["service"].(string)
		if a.Op == "messages.add_member" || a.Op == "messages.remove_member" || a.Op == "messages.rename_group" || a.Op == "messages.leave_group" {
			if c["style"] != "group" {
				return contracts.ErrUnsupported
			}
		}
		raw, _ := json.Marshal(c["participants"])
		var peers []struct {
			Ref struct {
				NativeKey string `json:"native_key"`
			} `json:"$ref"`
		}
		json.Unmarshal(raw, &peers)
		for _, p := range peers {
			if a.Origin != "interactive" && p.Ref.NativeKey != "message-self" && p.Ref.NativeKey != "message-self-email" && (a.Op == "messages.send" || a.Op == "messages.reply") {
				return contracts.ErrUnauthorized
			}
		}
		if a.Op == "messages.remove_member" {
			if len(peers) < 4 {
				return contracts.ErrUnsupported
			}
		}
		if a.Op == "messages.leave_group" && len(peers) < 3 {
			return contracts.ErrUnsupported
		}
	}
	switch a.Op {
	case "messages.edit", "messages.unsend":
		if service != "iMessage" || data["is_from_me"] != true {
			return contracts.ErrUnsupported
		}
		sent, _ := data["sent_at"].(string)
		at, err := time.Parse(time.RFC3339Nano, sent)
		if err != nil {
			return contracts.ErrInvalid
		}
		age := time.Since(at)
		limit := 15 * time.Minute
		if a.Op == "messages.unsend" {
			limit = 2 * time.Minute
		}
		if age < 0 || age >= limit {
			return contracts.ErrUnsupported
		}
		history, _ := data["edit_history"].([]any)
		if a.Op == "messages.edit" && len(history) >= 5 {
			return contracts.ErrUnsupported
		}
	case "messages.add_member", "messages.remove_member", "messages.leave_group", "messages.rename_group":
		if service != "iMessage" {
			return contracts.ErrUnsupported
		}
	}
	return nil
}
func (f *fakeApple) messageCommand(wire *applepeer.Wire, frame contracts.IPCFrame, fence uint64, emit func(contracts.Capture, uint64) error, checkpoint func() string) error {
	reject := func(e *contracts.Error) error {
		_, err := wire.Send(contracts.IPCFrame{T: "Rejected", CommandID: frame.CommandID, Disposition: contracts.DispositionRejectedBeforeDispatch, Error: e})
		return err
	}
	switch frame.T {
	case "Prepare":
		f.mu.Lock()
		if frame.Action == nil || frame.Principal == "" || frame.RequestHash == "" {
			f.mu.Unlock()
			return reject(contracts.ErrInvalid)
		}
		a := *frame.Action
		if frame.Origin != "" && frame.Origin != "interactive" || a.Origin != frame.Origin || frame.Origin == "interactive" && !(strings.HasPrefix(frame.Principal, "device:") || strings.HasPrefix(frame.Principal, "agent:") || strings.Contains(frame.Principal, "/device:")) {
			f.mu.Unlock()
			return reject(contracts.ErrInvalid)
		}
		if e := f.messageValidate(a); e != nil {
			f.mu.Unlock()
			return reject(e)
		}
		if f.fault == "ui-interference-before" {
			f.fault = ""
			f.mu.Unlock()
			return reject(contracts.ErrConflict)
		}
		before := f.records[a.NativeKey].Data
		if a.NativeKey != "" && len(frame.ExpectedState) > 0 {
			var expected, native map[string]any
			json.Unmarshal(frame.ExpectedState, &expected)
			json.Unmarshal(before, &native)
			for _, field := range []string{"text", "sent_at", "is_from_me", "edit_history", "retracted_parts", "display_name", "service"} {
				if value, ok := expected[field]; ok && !reflect.DeepEqual(value, native[field]) {
					f.mu.Unlock()
					return reject(contracts.ErrConflict)
				}
			}
		}
		for key, t := range f.tickets {
			if time.Now().After(t.Expires) {
				delete(f.tickets, key)
			} else {
				f.mu.Unlock()
				return reject(contracts.ErrUnavailable)
			}
		}
		key := contracts.NewID("ticket")
		expiry := time.Now().Add(30 * time.Second)
		if frame.ExpiresAt.Before(expiry) {
			expiry = frame.ExpiresAt
		}
		f.tickets[key] = ticket{CommandID: frame.CommandID, Principal: frame.Principal, Hash: frame.RequestHash, Action: a, Before: before, Expires: expiry, Fence: fence}
		maximum := len(f.records)
		f.mu.Unlock()
		_, err := wire.Send(contracts.IPCFrame{T: "Prepared", CommandID: frame.CommandID, Ticket: key, NativeState: before, Route: "sim-messages", Fingerprint: frame.RequestHash, CandidateIDs: []string{fmt.Sprint(maximum), a.ListKey}})
		return err
	case "Dispatch":
		f.mu.Lock()
		t, ok := f.tickets[frame.Ticket]
		delete(f.tickets, frame.Ticket)
		if !ok || frame.Origin != t.Action.Origin || t.CommandID != frame.CommandID || t.Fence != fence || frame.OwnerFence != fence || !time.Now().Before(t.Expires) {
			f.mu.Unlock()
			return reject(contracts.ErrExpired)
		}
		if e := f.messageValidate(t.Action); e != nil {
			f.mu.Unlock()
			return reject(e)
		}
		if !reflect.DeepEqual(t.Before, f.records[t.Action.NativeKey].Data) {
			f.mu.Unlock()
			return reject(contracts.ErrConflict)
		}
		if err := f.guard(guardEntry{CommandID: t.CommandID, Ticket: frame.Ticket, StartedAt: time.Now().UTC()}); err != nil {
			f.mu.Unlock()
			return reject(contracts.ErrUnavailable)
		}
		fault := f.fault
		f.fault = ""
		if fault == "ui-interference" {
			f.mu.Unlock()
			_, err := wire.Send(contracts.IPCFrame{T: "Outcome", CommandID: t.CommandID, Disposition: contracts.DispositionIndeterminate})
			return err
		}
		a := t.Action
		m := a.Messages
		now := time.Now().UTC()
		observations := []contracts.Observation{}
		put := func(collection, key string, data map[string]any, absent bool) {
			raw, _ := json.Marshal(data)
			kind := "upsert"
			if absent {
				kind = "absent"
			}
			o := contracts.Observation{Kind: kind, Collection: "messages." + collection, NativeKey: key, ScopedAlias: "messages." + collection + "/" + key, Data: raw, ObservedAt: now, Revalidated: true}
			f.records[key] = o
			observations = append(observations, o)
		}
		native := a.NativeKey
		collection := strings.TrimPrefix(contracts.MessagesTargetCollection(a.Op), "messages.")
		var data map[string]any
		json.Unmarshal(t.Before, &data)
		if data == nil {
			data = map[string]any{}
		}
		parts := []map[string]any{}
		disposition := contracts.DispositionLocallyObserved
		newMessage := func(key, text, chat string, fromMe bool) {
			put("message", key, map[string]any{"chat_id": messageReference("chat", chat), "text": text, "parts": []any{map[string]any{"index": 0, "kind": "text", "text": text}}, "is_from_me": fromMe, "sent_at": now, "decode_status": "decoded", "edit_history": []any{}, "retracted_parts": []any{}, "kind": "message", "error_code": 0}, false)
		}
		switch a.Op {
		case "messages.send", "messages.reply", "messages.create_group":
			chat := a.ListKey
			if a.Op == "messages.reply" {
				raw, _ := json.Marshal(data["chat_id"])
				var ref struct {
					Ref struct {
						NativeKey string `json:"native_key"`
					} `json:"$ref"`
				}
				json.Unmarshal(raw, &ref)
				chat = ref.Ref.NativeKey
			}
			if chat == "" {
				chat = contracts.NewID("chat")
				style := "direct"
				if a.Op == "messages.create_group" {
					style = "group"
				}
				peers := []any{}
				for _, h := range m.Handles {
					handle := simHandleKey(h)
					put("handle", handle, map[string]any{"address": h, "service": m.Service}, false)
					peers = append(peers, messageReference("handle", handle))
				}
				put("chat", chat, map[string]any{"service": m.Service, "style": style, "display_name": m.Name, "participants": peers}, false)
			}
			native = contracts.NewID("message")
			newMessage(native, m.Text, chat, true)
			if a.Op == "messages.reply" {
				o := f.records[native]
				var d map[string]any
				json.Unmarshal(o.Data, &d)
				d["thread_originator_id"] = messageReference("message", a.NativeKey)
				put("message", native, d, false)
			}
			if fault == "delivery-failed" {
				o := f.records[native]
				var data map[string]any
				json.Unmarshal(o.Data, &data)
				data["error_code"] = 100
				put("message", native, data, false)
				disposition = contracts.DispositionDeliveryFailed
			} else if fault == "ghost-send" {
				o := f.records[native]
				var d map[string]any
				json.Unmarshal(o.Data, &d)
				delete(d, "chat_id")
				d["text"] = ""
				d["parts"] = []any{}
				d["service"] = "SMS"
				put("message", native, d, false)
				disposition = contracts.DispositionDeliveryFailed
			} else {
				newMessage(native+"-echo", m.Text, chat, false)
				parts = append(parts, map[string]any{"index": 0, "kind": "text", "native_key": native, "disposition": "locally_observed"})
				for i, file := range m.Files {
					bytes, err := os.ReadFile(file.Path)
					if err != nil {
						f.mu.Unlock()
						return reject(contracts.ErrInvalid)
					}
					sum := sha256.Sum256(bytes)
					digest := fmt.Sprintf("%x", sum)
					if digest != file.Digest {
						f.mu.Unlock()
						return reject(contracts.ErrInvalid)
					}
					f.blobs[digest] = bytes
					key := contracts.NewID("attachment")
					msg := contracts.NewID("message")
					newMessage(msg, "", chat, true)
					derivatives, media := f.mediaVariants(file.Name, bytes)
					put("attachment", key, map[string]any{"message_id": messageReference("message", msg), "filename": file.Name, "size": len(bytes), "mime_type": simMediaMIME(file.Name, bytes), "blob": map[string]any{"digest": digest, "state": "present"}, "derivatives": derivatives, "media": media}, false)
					parts = append(parts, map[string]any{"index": i + 1, "kind": "attachment", "native_key": msg, "disposition": "locally_observed"})
				}
			}
			if fault == "identical-manual-send" {
				newMessage(native+"-manual", m.Text, chat, true)
				disposition = contracts.DispositionIndeterminate
			}
			if a.Op == "messages.create_group" {
				native = chat
				collection = "chat"
			}
		case "messages.edit":
			history, _ := data["edit_history"].([]any)
			history = append(history, map[string]any{"text": data["text"], "timestamp": now})
			data["edit_history"] = history
			data["text"] = m.Text
			data["edited_at"] = now
			put(collection, native, data, false)
		case "messages.unsend":
			data["retracted_at"] = now
			data["retracted_parts"] = []int{m.PartIndex}
			put(collection, native, data, false)
		case "messages.react":
			collection = "reaction"
			native = contracts.NewID("reaction")
			put(collection, native, map[string]any{"message_id": messageReference("message", a.NativeKey), "part_index": m.PartIndex, "kind": m.Reaction, "emoji": m.Emoji, "removed": m.Remove, "is_from_me": true}, false)
		case "messages.mark_read", "messages.mark_unread":
			data["is_unread"] = a.Op == "messages.mark_unread"
			data["last_read_at"] = now
			put(collection, native, data, false)
		case "messages.delete_message", "messages.delete_chat":
			put(collection, native, data, true)
		case "messages.rename_group":
			data["display_name"] = m.Name
			put(collection, native, data, false)
		case "messages.add_member", "messages.remove_member":
			peers, _ := data["participants"].([]any)
			handle := simHandleKey(m.Handle)
			if a.Op == "messages.add_member" {
				put("handle", handle, map[string]any{"address": m.Handle, "service": "iMessage"}, false)
			}
			if a.Op == "messages.add_member" {
				peers = append(peers, messageReference("handle", handle))
			} else {
				filtered := []any{}
				for _, p := range peers {
					raw, _ := json.Marshal(p)
					want, _ := json.Marshal(messageReference("handle", handle))
					if string(raw) != string(want) {
						filtered = append(filtered, p)
					}
				}
				peers = filtered
			}
			data["participants"] = peers
			put(collection, native, data, false)
		case "messages.leave_group":
			data["left"] = true
			put(collection, native, data, false)
		}
		f.effects[t.CommandID]++
		evidence := map[string]any{"source": "messages", "collection": "messages." + collection, "native_key": native, "fingerprint": t.Hash, "observed": disposition != contracts.DispositionIndeterminate, "parts": parts, "candidate_count": 1}
		if fault == "ghost-send" {
			evidence["reason"] = "empty_unjoined_sms"
		}
		if fault == "identical-manual-send" {
			evidence["candidate_count"] = 2
		}
		raw, _ := json.Marshal(evidence)
		outcome := contracts.IPCFrame{T: "Outcome", CommandID: t.CommandID, Disposition: disposition, Evidence: raw}
		f.evidence[t.CommandID] = outcome
		err := f.changedLocked(contracts.Messages)
		revision := f.durable.Revision[contracts.Messages]
		f.mu.Unlock()
		if err != nil {
			return err
		}
		capture := contracts.Capture{Source: contracts.Messages, Observations: observations, BlobHandles: f.blobHandles()}
		if strings.HasPrefix(a.Op, "messages.delete_") {
			capture = fullScanSource(observations, contracts.Messages)
		}
		if err = emit(capture, revision); err != nil {
			return err
		}
		evidence["capture_checkpoint"] = checkpoint()
		if fault == "receipt-before-outcome" {
			f.mu.Lock()
			receipt := f.records[native]
			var data map[string]any
			json.Unmarshal(receipt.Data, &data)
			data["delivered_at"] = time.Now().UTC()
			receipt.Data, _ = json.Marshal(data)
			receipt.ObservedAt = time.Now().UTC()
			f.records[native] = receipt
			err = f.changedLocked(contracts.Messages)
			revision = f.durable.Revision[contracts.Messages]
			f.mu.Unlock()
			if err != nil {
				return err
			}
			if err = emit(contracts.Capture{Source: contracts.Messages, Observations: []contracts.Observation{receipt}}, revision); err != nil {
				return err
			}
		}
		outcome.Evidence, _ = json.Marshal(evidence)
		f.mu.Lock()
		f.evidence[t.CommandID] = outcome
		err = f.saveLocked()
		f.mu.Unlock()
		if err != nil {
			return err
		}
		if err = f.guard(guardEntry{CommandID: t.CommandID, Outcome: &outcome}); err != nil {
			return err
		}
		_, err = wire.Send(outcome)
		return err
	case "Resolve":
		f.mu.Lock()
		outcome := f.evidence[frame.CommandID]
		f.mu.Unlock()
		if outcome.Disposition == contracts.DispositionLocallyObserved {
			var e map[string]any
			json.Unmarshal(outcome.Evidence, &e)
			e["delivery_evidence"] = "sim_is_delivered"
			outcome.Disposition = contracts.DispositionDelivered
			outcome.Evidence, _ = json.Marshal(e)
		}
		_, err := wire.Send(outcome)
		return err
	}
	return contracts.ErrInvalid
}
