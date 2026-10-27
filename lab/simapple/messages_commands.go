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
	for _, op := range contracts.MessageVariantCapabilities {
		c.Operations = append(c.Operations, contracts.OperationCapability{Operation: op, Proven: false, Reason: "per_os_variant_proof_required"})
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
	if m != nil && (m.Variant != nil || a.Op == "messages.keep_audio") {
		return messageProblem("capability_unproven")
	}
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
		if file.Path == "" || file.Digest == "" || !messageUploadBoundary(file.Path) {
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
	if problem := f.messageParityValidate(a, data); problem != nil {
		return problem
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
		if !actionWindowOpen(data, a.Op == "messages.edit", time.Now()) {
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
		before := f.messageBefore(a)
		if a.NativeKey != "" && len(frame.ExpectedState) > 0 {
			var expected, native map[string]any
			json.Unmarshal(frame.ExpectedState, &expected)
			json.Unmarshal(f.records[a.NativeKey].Data, &native)
			for _, field := range []string{"text", "sent_at", "is_from_me", "edit_history", "retracted_parts", "display_name", "service", "pinned_index", "muted", "read_receipts", "deleted_at"} {
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
		if !reflect.DeepEqual(t.Before, f.messageBefore(t.Action)) {
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
			if !absent {
				f.observationFacts(collection, data)
			}
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
		json.Unmarshal(f.records[a.NativeKey].Data, &data)
		if data == nil {
			data = map[string]any{}
		}
		parts := []map[string]any{}
		disposition := contracts.DispositionLocallyObserved
		newMessage := func(key, text, chat string, fromMe bool) {
			put("message", key, map[string]any{"chat_id": messageReference("chat", chat), "text": text, "parts": []any{map[string]any{"index": 0, "kind": "text", "text": text}}, "is_from_me": fromMe, "sent_at": now, "decode_status": "decoded", "edit_history": []any{}, "retracted_parts": []any{}, "kind": "message", "error_code": 0}, false)
		}
		switch a.Op {
		case "messages.pin", "messages.mute", "messages.set_read_receipts", "messages.recover_chat", "messages.delete_chat_permanently":
			f.messageParityApply(a, now, put)
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
			hasText := m.Text != "" || a.Op != "messages.send" || len(m.Files) == 0
			if hasText || fault == "delivery-failed" || fault == "ghost-send" {
				newMessage(native, m.Text, chat, true)
			} else {
				native = ""
			}
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
				parts = append(parts, map[string]any{"index": 0, "kind": "text", "disposition": "delivery_failed", "observed": false, "reason": "empty_unjoined_sms"})
			} else {
				if hasText {
					newMessage(native+"-echo", m.Text, chat, false)
					parts = append(parts, map[string]any{"index": 0, "kind": "text", "native_key": native, "disposition": "locally_observed", "observed": true})
				}
				for fileIndex, file := range m.Files {
					if (fault == "part-no-effect" || fault == "part-progress-failure") && fileIndex == len(m.Files)-1 {
						parts = append(parts, map[string]any{"index": len(parts), "kind": "attachment", "disposition": "failed_no_effect", "observed": false})
						disposition = contracts.DispositionDeliveryFailed
						continue
					}
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
					put("message", msg, map[string]any{"chat_id": messageReference("chat", chat), "text": "", "parts": []any{map[string]any{"index": 0, "kind": "attachment", "attachment_id": messageReference("attachment", key)}}, "is_from_me": true, "sent_at": now, "decode_status": "decoded", "kind": "message", "error_code": 0}, false)
					if native == "" {
						native = msg
					}
					derivatives, media := f.mediaVariants(file.Name, bytes)
					put("attachment", key, map[string]any{"message_id": messageReference("message", msg), "filename": file.Name, "size": len(bytes), "mime_type": simMediaMIME(file.Name, bytes), "blob": map[string]any{"digest": digest, "state": "present"}, "derivatives": derivatives, "media": media}, false)
					parts = append(parts, map[string]any{"index": len(parts), "kind": "attachment", "native_key": msg, "disposition": "locally_observed", "observed": true})
				}
				if fault == "part-delivery-failed" && len(parts) > 0 {
					failed := parts[len(parts)-1]
					key, _ := failed["native_key"].(string)
					var data map[string]any
					json.Unmarshal(f.records[key].Data, &data)
					data["error_code"] = 100
					put("message", key, data, false)
					failed["disposition"] = "delivery_failed"
					disposition = contracts.DispositionDeliveryFailed
				}
			}
			if fault == "part-progress" || fault == "part-progress-failure" {
				for _, part := range parts {
					if part["disposition"] == "failed_no_effect" {
						continue
					}
					part["observed"] = false
					part["disposition"] = "indeterminate"
				}
				if fault == "part-progress" {
					disposition = contracts.DispositionIndeterminate
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
			parts, _ := data["parts"].([]any)
			for _, value := range parts {
				part, _ := value.(map[string]any)
				index, ok := part["index"].(float64)
				if ok && index == float64(m.PartIndex) && part["kind"] == "text" {
					part["text"] = m.Text
				}
			}
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
			for key, observation := range f.records {
				if a.Op != "messages.mark_read" || observation.Collection != "messages.message" || observation.Kind == "absent" {
					continue
				}
				var message map[string]any
				json.Unmarshal(observation.Data, &message)
				reference, _ := message["chat_id"].(map[string]any)
				ref, _ := reference["$ref"].(map[string]any)
				if ref["native_key"] != native || message["is_from_me"] == true || message["read_at"] != nil {
					continue
				}
				message["read_at"] = now
				put("message", key, message, false)
			}
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
		if fault == "outcome-without-capture" {
			evidence["reason"] = "capture_omitted"
		}
		if fault == "part-progress" || fault == "part-progress-failure" {
			evidence["reason"] = "part_progress"
			evidence["observed"] = false
		}
		if fault == "ghost-send" {
			evidence["reason"] = "empty_unjoined_sms"
			evidence["observed"] = false
			delete(evidence, "native_key")
		}
		if fault == "identical-manual-send" {
			evidence["candidate_count"] = 2
		}
		raw, _ := json.Marshal(evidence)
		outcome := contracts.IPCFrame{T: "Outcome", CommandID: t.CommandID, Disposition: disposition, Evidence: raw}
		f.evidence[t.CommandID] = outcome
		err := f.changedLocked(contracts.Messages)
		revision := f.durable.Revision[contracts.Messages]
		var purgeSnapshot []contracts.Observation
		if a.Op == "messages.delete_chat_permanently" {
			for _, observation := range f.records {
				purgeSnapshot = append(purgeSnapshot, observation)
			}
		}
		f.mu.Unlock()
		if err != nil {
			return err
		}
		capture := contracts.Capture{Source: contracts.Messages, Observations: observations, BlobHandles: f.blobHandles()}
		if a.Op == "messages.delete_chat_permanently" {
			capture = fullScanSource(purgeSnapshot, contracts.Messages)
			capture.BlobHandles = f.blobHandles()
		} else if strings.HasPrefix(a.Op, "messages.delete_") {
			capture = fullScanSource(observations, contracts.Messages)
		}
		if fault == "outcome-without-capture" {
			_, err = wire.Send(outcome)
			return err
		}
		if fault == "outcome-before-capture" {
			if _, err = wire.Send(outcome); err != nil {
				return err
			}
			time.Sleep(300 * time.Millisecond)
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
		if fault == "outcome-before-capture" {
			return nil
		}
		if err = f.guard(guardEntry{CommandID: t.CommandID, Outcome: &outcome}); err != nil {
			return err
		}
		_, err = wire.Send(outcome)
		return err
	case "Resolve":
		f.mu.Lock()
		outcome := f.evidence[frame.CommandID]
		stale := f.fault == "part-progress-stale"
		receiptFirst := f.fault == "part-receipt-failed-first"
		receiptFailed := f.fault == "part-receipt-failed" || receiptFirst
		unboundFailure := f.fault == "part-unbound-failure"
		if stale || receiptFailed || unboundFailure {
			f.fault = ""
		}
		f.mu.Unlock()
		if receiptFailed {
			var evidence map[string]any
			json.Unmarshal(outcome.Evidence, &evidence)
			parts, _ := evidence["parts"].([]any)
			if len(parts) > 0 {
				index := len(parts) - 1
				if receiptFirst {
					index = 0
				}
				part, _ := parts[index].(map[string]any)
				key, _ := part["native_key"].(string)
				f.mu.Lock()
				record := f.records[key]
				var data map[string]any
				json.Unmarshal(record.Data, &data)
				data["error_code"] = 100
				record.Data, _ = json.Marshal(data)
				record.ObservedAt = time.Now().UTC()
				f.records[key] = record
				err := f.changedLocked(contracts.Messages)
				revision := f.durable.Revision[contracts.Messages]
				f.mu.Unlock()
				if err != nil {
					return err
				}
				if err = emit(contracts.Capture{Source: contracts.Messages, Observations: []contracts.Observation{record}}, revision); err != nil {
					return err
				}
				part["disposition"] = "delivery_failed"
				evidence["capture_checkpoint"] = checkpoint()
				outcome.Disposition = contracts.DispositionDeliveryFailed
				outcome.Evidence, _ = json.Marshal(evidence)
			}
		}
		if unboundFailure {
			var evidence map[string]any
			json.Unmarshal(outcome.Evidence, &evidence)
			parts, _ := evidence["parts"].([]any)
			if len(parts) > 0 {
				part, _ := parts[len(parts)-1].(map[string]any)
				delete(part, "native_key")
				part["observed"] = false
				part["disposition"] = "delivery_failed"
				outcome.Disposition = contracts.DispositionDeliveryFailed
				outcome.Evidence, _ = json.Marshal(evidence)
			}
		}
		var progress map[string]any
		json.Unmarshal(outcome.Evidence, &progress)
		if progress["reason"] == "capture_omitted" {
			progress["capture_checkpoint"] = checkpoint()
			outcome.Evidence, _ = json.Marshal(progress)
		}
		if progress["reason"] == "part_progress" && !stale {
			parts, _ := progress["parts"].([]any)
			if len(parts) > 0 {
				part, _ := parts[0].(map[string]any)
				part["observed"] = true
				part["disposition"] = "locally_observed"
				progress["observed"] = true
				outcome.Evidence, _ = json.Marshal(progress)
			}
		}
		if outcome.Disposition == contracts.DispositionLocallyObserved {
			var e map[string]any
			json.Unmarshal(outcome.Evidence, &e)
			e["delivery_evidence"] = "sim_is_delivered"
			if parts, ok := e["parts"].([]any); ok {
				for _, value := range parts {
					if part, ok := value.(map[string]any); ok && part["disposition"] == "locally_observed" {
						part["disposition"] = "delivered"
					}
				}
			}
			outcome.Disposition = contracts.DispositionDelivered
			outcome.Evidence, _ = json.Marshal(e)
		}
		_, err := wire.Send(outcome)
		return err
	}
	return contracts.ErrInvalid
}
