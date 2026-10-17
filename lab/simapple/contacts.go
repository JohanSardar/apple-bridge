package simapple

import (
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"net/http"
	"os"
	"path/filepath"
	"reflect"
	"strconv"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/applepeer"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func contactsScan(records []contracts.Observation) contracts.Capture {
	return contracts.Capture{Source: contracts.Contacts, Observations: records, Scope: []string{"contacts.contact", "contacts.group"}, ScanID: contracts.NewID("scan"), ScanStartedAt: time.Now().UTC(), CompletionEvidence: "complete"}
}
func (f *fakeApple) contactSnapshot(drop bool) contracts.Capture {
	f.mu.Lock()
	defer f.mu.Unlock()
	observations := []contracts.Observation{}
	for _, o := range f.records {
		if contracts.CollectionSource(o.Collection) == contracts.Contacts && (!drop || o.Kind != "absent") {
			observations = append(observations, o)
		}
	}
	capture := contactsScan(observations)
	capture.Generation = f.contactGeneration()
	capture.Checkpoint = "contacts-history-" + strconv.FormatUint(f.contactSequence, 10)
	capture.BlobHandles = map[string]string{}
	for digest := range f.blobs {
		capture.BlobHandles[digest] = digest
	}
	return capture
}

func (f *fakeApple) contactToken() string {
	f.mu.Lock()
	defer f.mu.Unlock()
	return "contacts-history-" + strconv.FormatUint(f.contactSequence, 10)
}

func (f *fakeApple) contactGeneration() string {
	b, err := os.ReadFile(filepath.Join(f.home, "contacts-generation"))
	if err == nil {
		return string(b)
	}
	return "contacts-v1"
}
func (f *fakeApple) contactsControl(mux *http.ServeMux) {
	mux.HandleFunc("POST /contacts", func(w http.ResponseWriter, r *http.Request) {
		var a struct {
			Action     string         `json:"action"`
			Key        string         `json:"key"`
			UnifiedKey string         `json:"unified_key"`
			Data       map[string]any `json:"data"`
			Readonly   bool           `json:"readonly"`
		}
		if json.NewDecoder(r.Body).Decode(&a) != nil {
			http.Error(w, "invalid", 400)
			return
		}
		f.mu.Lock()
		if a.UnifiedKey != "" {
			a.Key = a.UnifiedKey
		}
		records := []contracts.Observation{}
		switch a.Action {
		case "drop-everything":
			if os.WriteFile(filepath.Join(f.home, "contacts-generation"), []byte(contracts.NewID("contacts")), 0600) != nil {
				f.mu.Unlock()
				http.Error(w, "persist", 500)
				return
			}
			for _, o := range f.records {
				if contracts.CollectionSource(o.Collection) == contracts.Contacts && o.Kind != "absent" {
					records = append(records, o)
				}
			}

		case "group":
			raw, _ := json.Marshal(map[string]any{"name": "Bridge Test", "member_ids": []any{map[string]any{"$ref": map[string]string{"collection": "contacts.contact", "native_key": a.Key}}}})
			key := "group-" + a.Key
			f.records[key] = contracts.Observation{Kind: "upsert", Collection: "contacts.group", NativeKey: key, ScopedAlias: "contacts/contacts.group/" + key, Data: raw, ObservedAt: time.Now().UTC()}
		case "create", "update", "image", "delete", "token-crash":
			if a.Key == "" {
				f.mu.Unlock()
				http.Error(w, "invalid", 400)
				return
			}
			data := map[string]any{}
			if old, ok := f.records[a.Key]; ok {
				decodeNative(old.Data, &data)
			}
			for k, v := range a.Data {
				if a.UnifiedKey != "" && (k == "phones" || k == "emails") {
					existing, _ := data[k].([]any)
					added, _ := v.([]any)
					data[k] = append(existing, added...)
				} else {
					data[k] = v
				}
			}
			data["note"] = "unavailable"
			data["writable"] = !a.Readonly
			data["container_id"] = "default"
			if a.Readonly {
				data["container_id"] = "readonly"
			}
			if a.Action == "image" {
				b, err := base64.StdEncoding.DecodeString(a.Data["base64"].(string))
				if err != nil {
					f.mu.Unlock()
					http.Error(w, "invalid", 400)
					return
				}
				delete(data, "base64")
				sum := sha256.Sum256(b)
				digest := hex.EncodeToString(sum[:])
				f.blobs[digest] = b
				data["image"] = map[string]any{"digest": digest, "state": "present"}
			}
			raw, _ := json.Marshal(data)
			o := contracts.Observation{Kind: "upsert", Collection: "contacts.contact", NativeKey: a.Key, ScopedAlias: "contacts/contacts.contact/" + a.Key, Data: raw, ObservedAt: time.Now().UTC()}
			if a.Action == "delete" {
				o.Kind = "absent"
				o.Revalidated = true
			}
			f.records[a.Key] = o
			records = append(records, o)
		default:
			f.mu.Unlock()
			http.Error(w, "invalid", 400)
			return
		}
		f.contactSequence++
		if err := f.changedLocked(contracts.Contacts); err != nil {
			f.mu.Unlock()
			http.Error(w, "persist", 500)
			return
		}
		missing := f.contactPermissionMissing
		revision := f.durable.Revision[contracts.Contacts]
		f.mu.Unlock()
		if !missing {
			capture := f.contactSnapshot(a.Action == "drop-everything")
			if a.Action == "token-crash" {
				os.Exit(78)
			}
			if err := f.enqueue(r.Context(), capture, true, revision); err != nil {
				return
			}
		}
		json.NewEncoder(w).Encode(map[string]bool{"effect_done": true})
	})
}
func (f *fakeApple) isContactCommand(frame contracts.IPCFrame) bool {
	if frame.Action != nil && strings.HasPrefix(frame.Action.Op, "contacts.") {
		return true
	}
	f.mu.Lock()
	defer f.mu.Unlock()
	if t, ok := f.tickets[frame.Ticket]; ok {
		return strings.HasPrefix(t.Action.Op, "contacts.")
	}
	if e, ok := f.evidence[frame.CommandID]; ok {
		var v map[string]any
		json.Unmarshal(e.Evidence, &v)
		return v["source"] == "contacts"
	}
	return false
}
func (f *fakeApple) contactCommand(wire *applepeer.Wire, frame contracts.IPCFrame, fence uint64, emit func(contracts.Capture) error, checkpoint func(contracts.Source) string) error {
	reject := func(e *contracts.Error) error {
		_, err := wire.Send(contracts.IPCFrame{T: "Rejected", CommandID: frame.CommandID, Error: e, Disposition: "rejected_before_dispatch"})
		return err
	}
	rejectConflict := func(key string) error {
		if err := emit(f.contactSnapshot(false)); err != nil {
			return err
		}
		evidence, _ := json.Marshal(contracts.ConflictObservation{NativeKey: key, CaptureCheckpoint: checkpoint(contracts.Contacts)})
		_, err := wire.Send(contracts.IPCFrame{T: "Rejected", CommandID: frame.CommandID, Disposition: contracts.DispositionRejectedBeforeDispatch, Error: contracts.ErrConflict, Evidence: evidence})
		return err
	}
	switch frame.T {
	case "Prepare":
		f.mu.Lock()
		a := *frame.Action
		before := json.RawMessage(nil)
		if f.contactPermissionMissing {
			f.mu.Unlock()
			return reject(contracts.ErrUnavailable)
		}
		if a.NativeKey != "" {
			o, ok := f.records[a.NativeKey]
			if !ok {
				f.mu.Unlock()
				return reject(contracts.ErrNotFound)
			}
			if o.Kind == "absent" {
				f.mu.Unlock()
				return rejectConflict(a.NativeKey)
			}
			before = o.Data
			var current, expected map[string]any
			decodeNative(before, &current)
			decodeNative(frame.ExpectedState, &expected)
			if !reflect.DeepEqual(current, expected) {
				f.mu.Unlock()
				return rejectConflict(a.NativeKey)
			}
			if current["writable"] != true {
				f.mu.Unlock()
				return reject(contracts.ErrUnsupported)
			}
		}
		key := contracts.NewID("ticket")
		expiry := time.Now().Add(30 * time.Second)
		if frame.ExpiresAt.Before(expiry) {
			expiry = frame.ExpiresAt
		}
		f.tickets[key] = ticket{CommandID: frame.CommandID, Principal: frame.Principal, Hash: frame.RequestHash, Action: a, Before: before, Fence: fence, Expires: expiry}
		f.mu.Unlock()
		_, err := wire.Send(contracts.IPCFrame{T: "Prepared", CommandID: frame.CommandID, Ticket: key, NativeState: before, Route: "sim-contacts", Fingerprint: frame.RequestHash})
		return err
	case "Dispatch":
		f.mu.Lock()
		t, ok := f.tickets[frame.Ticket]
		delete(f.tickets, frame.Ticket)
		if !ok || t.CommandID != frame.CommandID || t.Fence != fence || frame.OwnerFence != fence || !time.Now().Before(t.Expires) {
			f.mu.Unlock()
			return reject(contracts.ErrExpired)
		}
		if f.contactPermissionMissing {
			f.mu.Unlock()
			return reject(contracts.ErrUnavailable)
		}
		if t.Action.NativeKey != "" && (f.records[t.Action.NativeKey].Kind == "absent" || string(f.records[t.Action.NativeKey].Data) != string(t.Before)) {
			f.mu.Unlock()
			return rejectConflict(t.Action.NativeKey)
		}
		if err := f.guard(guardEntry{CommandID: t.CommandID, Ticket: frame.Ticket, StartedAt: time.Now().UTC()}); err != nil {
			f.mu.Unlock()
			return err
		}
		key := t.Action.NativeKey
		if key == "" {
			key = contracts.NewID("contact")
		}
		data := map[string]any{"note": "unavailable", "container_id": "default", "writable": true}
		if t.Before != nil {
			decodeNative(t.Before, &data)
		}
		raw, _ := json.Marshal(t.Action.Args)
		var patch map[string]any
		decodeNative(raw, &patch)
		delete(patch, "id")
		for k, v := range patch {
			data[k] = v
		}
		if image, ok := patch["image"].(map[string]any); ok {
			encoded, _ := image["base64"].(string)
			bytes, err := base64.StdEncoding.DecodeString(encoded)
			if err != nil {
				f.mu.Unlock()
				return reject(contracts.ErrInvalid)
			}
			sum := sha256.Sum256(bytes)
			digest := hex.EncodeToString(sum[:])
			f.blobs[digest] = bytes
			data["image"] = map[string]any{"digest": digest, "state": "present"}
		}

		raw, _ = json.Marshal(data)
		o := contracts.Observation{Kind: "upsert", Collection: "contacts.contact", NativeKey: key, ScopedAlias: "contacts/contacts.contact/" + key, Data: raw, ObservedAt: time.Now().UTC()}
		if t.Action.Op == "contacts.delete" {
			o.Kind = "absent"
			o.Revalidated = true
		}
		f.records[key] = o
		f.contactSequence++
		f.effects[t.CommandID]++
		evidence, _ := json.Marshal(map[string]any{"source": "contacts", "native_key": key, "observed": true, "absent": o.Kind == "absent", "fingerprint": t.Hash, "observed_digest": contactDigest(o.Data)})
		outcome := contracts.IPCFrame{T: "Outcome", CommandID: t.CommandID, Disposition: "locally_observed", Evidence: evidence}
		f.evidence[t.CommandID] = outcome
		fault := f.fault
		f.fault = ""
		err := f.changedLocked(contracts.Contacts)
		f.mu.Unlock()
		if err != nil {
			return err
		}
		if fault == "crash-after-effect" {
			os.Exit(78)
		}
		c := f.contactSnapshot(false)
		if err = emit(c); err != nil {
			return err
		}
		var values map[string]any
		json.Unmarshal(evidence, &values)
		values["capture_checkpoint"] = checkpoint(contracts.Contacts)
		outcome.Evidence, _ = json.Marshal(values)
		f.mu.Lock()
		f.evidence[t.CommandID] = outcome
		err = f.saveLocked()
		f.mu.Unlock()
		if err != nil {
			return err
		}
		if fault == "crash-before-outcome" {
			os.Exit(78)
		}
		if fault == "ambiguous-outcome" {
			outcome.Disposition = "indeterminate"
		}
		if err = f.guard(guardEntry{CommandID: t.CommandID, Outcome: &outcome}); err != nil {
			return err
		}
		_, err = wire.Send(outcome)
		return err
	case "Resolve":
		f.mu.Lock()
		outcome, exists := f.evidence[frame.CommandID]
		var evidence map[string]any
		if json.Unmarshal(outcome.Evidence, &evidence) != nil {
			exists = false
		}
		key, _ := evidence["native_key"].(string)
		absent, _ := evidence["absent"].(bool)
		record, found := f.records[key]
		matches := exists && found && (record.Kind == "absent") == absent
		if !absent {
			matches = matches && evidence["observed_digest"] == contactDigest(record.Data)
		}
		f.mu.Unlock()
		if matches {
			if err := emit(f.contactSnapshot(false)); err != nil {
				return err
			}
			evidence["capture_checkpoint"] = checkpoint(contracts.Contacts)
			outcome.Evidence, _ = json.Marshal(evidence)
			outcome.Disposition = "locally_observed"
		} else {
			outcome = contracts.IPCFrame{T: "Outcome", CommandID: frame.CommandID, Disposition: "indeterminate"}
		}
		if matches {
			if err := f.guard(guardEntry{CommandID: frame.CommandID, Outcome: &outcome}); err != nil {
				return err
			}
		}
		_, err := wire.Send(outcome)
		return err
	}
	return contracts.ErrInvalid
}

func contactDigest(data []byte) string { sum := sha256.Sum256(data); return hex.EncodeToString(sum[:]) }
