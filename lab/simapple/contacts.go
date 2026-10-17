package simapple

import (
	"crypto/sha256"
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
			Action   string         `json:"action"`
			Key      string         `json:"key"`
			Data     map[string]any `json:"data"`
			Readonly bool           `json:"readonly"`
		}
		if json.NewDecoder(r.Body).Decode(&a) != nil {
			http.Error(w, "invalid", 400)
			return
		}
		f.mu.Lock()
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
				data[k] = v
			}
			data["note"] = "unavailable"
			data["writable"] = !a.Readonly
			data["container_id"] = "default"
			if a.Readonly {
				data["container_id"] = "readonly"
			}
			if a.Action == "image" {
				b := []byte("[ab-test] contact image bytes")
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
		if err := f.saveLocked(); err != nil {
			f.mu.Unlock()
			http.Error(w, "persist", 500)
			return
		}
		missing := f.permissionMissing
		f.mu.Unlock()
		if !missing {
			capture := f.contactSnapshot(a.Action == "drop-everything")
			if a.Action == "token-crash" {
				os.Exit(78)
			}
			if err := f.enqueue(r.Context(), capture, true); err != nil {
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
	switch frame.T {
	case "Prepare":
		f.mu.Lock()
		a := *frame.Action
		before := json.RawMessage(nil)
		if f.permissionMissing {
			f.mu.Unlock()
			return reject(contracts.ErrUnavailable)
		}
		if a.NativeKey != "" {
			o, ok := f.records[a.NativeKey]
			if !ok || o.Kind == "absent" {
				f.mu.Unlock()
				return reject(contracts.ErrNotFound)
			}
			before = o.Data
			var current, expected map[string]any
			decodeNative(before, &current)
			decodeNative(frame.ExpectedState, &expected)
			if !reflect.DeepEqual(current, expected) {
				f.mu.Unlock()
				return reject(contracts.ErrConflict)
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
		if f.permissionMissing {
			f.mu.Unlock()
			return reject(contracts.ErrUnavailable)
		}
		if t.Action.NativeKey != "" && string(f.records[t.Action.NativeKey].Data) != string(t.Before) {
			f.mu.Unlock()
			return reject(contracts.ErrConflict)
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
		raw, _ = json.Marshal(data)
		o := contracts.Observation{Kind: "upsert", Collection: "contacts.contact", NativeKey: key, ScopedAlias: "contacts/contacts.contact/" + key, Data: raw, ObservedAt: time.Now().UTC()}
		if t.Action.Op == "contacts.delete" {
			o.Kind = "absent"
			o.Revalidated = true
		}
		f.records[key] = o
		f.contactSequence++
		f.effects[t.CommandID]++
		evidence, _ := json.Marshal(map[string]any{"source": "contacts", "native_key": key, "observed": true, "absent": o.Kind == "absent", "fingerprint": t.Hash})
		outcome := contracts.IPCFrame{T: "Outcome", CommandID: t.CommandID, Disposition: "locally_observed", Evidence: evidence}
		f.evidence[t.CommandID] = outcome
		err := f.saveLocked()
		f.mu.Unlock()
		if err != nil {
			return err
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
		if err = f.guard(guardEntry{CommandID: t.CommandID, Outcome: &outcome}); err != nil {
			return err
		}
		_, err = wire.Send(outcome)
		return err
	case "Resolve":
		f.mu.Lock()
		outcome := f.evidence[frame.CommandID]
		f.mu.Unlock()
		_, err := wire.Send(outcome)
		return err
	}
	return contracts.ErrInvalid
}
