package simapple

import (
	"bufio"
	"bytes"
	"encoding/json"
	"net/http"
	"os"
	"path/filepath"
	"reflect"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/applepeer"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

type ticket struct {
	CommandID string
	Principal string
	Hash      string
	Action    contracts.NativeAction
	Before    json.RawMessage
	Expires   time.Time
	Fence     uint64
}
type guardEntry struct {
	CommandID string              `json:"command_id"`
	Ticket    string              `json:"ticket"`
	StartedAt time.Time           `json:"started_at"`
	Outcome   *contracts.IPCFrame `json:"outcome,omitempty"`
}
type durableApple struct {
	Records  map[string]contracts.Observation `json:"records"`
	Effects  map[string]int                   `json:"effects"`
	Evidence map[string]contracts.IPCFrame    `json:"evidence"`
}

func (f *fakeApple) saveLocked() error {
	b, err := json.Marshal(durableApple{f.records, f.effects, f.evidence})
	if err != nil {
		return err
	}
	file, err := os.OpenFile(filepath.Join(f.home, "truth.tmp"), os.O_CREATE|os.O_TRUNC|os.O_WRONLY, 0600)
	if err != nil {
		return err
	}
	_, err = file.Write(b)
	if err == nil {
		err = file.Sync()
	}
	file.Close()
	if err != nil {
		return err
	}
	if err = os.Rename(filepath.Join(f.home, "truth.tmp"), filepath.Join(f.home, "truth.json")); err != nil {
		return err
	}
	dir, err := os.Open(f.home)
	if err != nil {
		return err
	}
	defer dir.Close()
	return dir.Sync()
}
func (f *fakeApple) guard(entry guardEntry) error {
	file, err := os.OpenFile(filepath.Join(f.home, "dispatch-guard.log"), os.O_CREATE|os.O_APPEND|os.O_WRONLY, 0600)
	if err != nil {
		return err
	}
	defer file.Close()
	if err = json.NewEncoder(file).Encode(entry); err != nil {
		return err
	}
	return file.Sync()
}
func (f *fakeApple) commandControl(mux *http.ServeMux) {
	mux.HandleFunc("GET /effects", func(w http.ResponseWriter, r *http.Request) {
		f.mu.Lock()
		defer f.mu.Unlock()
		json.NewEncoder(w).Encode(f.effects)
	})
	mux.HandleFunc("POST /fault", func(w http.ResponseWriter, r *http.Request) {
		var fault struct {
			Name string `json:"name"`
			MS   int    `json:"ms"`
		}
		if json.NewDecoder(r.Body).Decode(&fault) != nil {
			http.Error(w, "invalid", 400)
			return
		}
		f.mu.Lock()
		f.fault = fault.Name
		f.pause = time.Duration(fault.MS) * time.Millisecond
		f.mu.Unlock()
		json.NewEncoder(w).Encode(map[string]bool{"armed": true})
	})
}
func (f *fakeApple) command(wire *applepeer.Wire, frame contracts.IPCFrame, fence uint64, emit func(contracts.Capture) error) error {
	reject := func(problem *contracts.Error) error {
		_, err := wire.Send(contracts.IPCFrame{T: "Rejected", CommandID: frame.CommandID, Disposition: "rejected_before_dispatch", Error: problem})
		return err
	}
	switch frame.T {
	case "Prepare":
		if frame.Action == nil || frame.Principal == "" || frame.RequestHash == "" {
			return reject(contracts.ErrInvalid)
		}
		switch frame.Action.Op {
		case "reminders.create", "reminders.create_list", "reminders.update", "reminders.complete", "reminders.uncomplete", "reminders.delete":
		default:
			return reject(contracts.ErrUnsupported)
		}
		f.mu.Lock()
		a := *frame.Action
		var before json.RawMessage
		if a.NativeKey != "" {
			o, ok := f.records[a.NativeKey]
			if !ok || o.Kind == "absent" {
				f.mu.Unlock()
				return reject(contracts.ErrNotFound)
			}
			before = o.Data
			var expected, native map[string]any
			if len(frame.ExpectedState) > 0 {
				decodeNative(frame.ExpectedState, &expected)
				decodeNative(before, &native)
				delete(expected, "list_id")
				delete(native, "list_id")
				if !reflect.DeepEqual(expected, native) {
					f.mu.Unlock()
					return reject(contracts.ErrConflict)
				}
			}
		}
		if a.Op != "reminders.create_list" {
			list, ok := f.records[a.ListKey]
			var data struct {
				Writable bool `json:"writable"`
			}
			json.Unmarshal(list.Data, &data)
			if !ok || !data.Writable {
				f.mu.Unlock()
				return reject(contracts.ErrUnsupported)
			}
		}
		for key, t := range f.tickets {
			if !time.Now().Before(t.Expires) {
				delete(f.tickets, key)
				continue
			}
			if (a.NativeKey != "" && a.NativeKey == t.Action.NativeKey) || (a.NativeKey == "" && t.Action.NativeKey == "" && a.ListKey == t.Action.ListKey) {
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
		fault, pause := f.fault, f.pause
		if fault == "pause" || fault == "external-edit-between-prepare-and-dispatch" {
			f.fault = ""
		}
		var changed *contracts.Observation
		if fault == "external-edit-between-prepare-and-dispatch" && a.NativeKey != "" {
			o := f.records[a.NativeKey]
			var data map[string]any
			decodeNative(o.Data, &data)
			data["title"] = "[ab-test] external race"
			o.Data, _ = json.Marshal(data)
			o.ObservedAt = time.Now().UTC()
			f.records[a.NativeKey] = o
			f.saveLocked()
			changed = &o
		}
		f.mu.Unlock()
		if changed != nil {
			if err := emit(contracts.Capture{Source: contracts.Reminders, Observations: []contracts.Observation{*changed}}); err != nil {
				return err
			}
		}
		if fault == "pause" {
			time.Sleep(pause)
		}
		_, err := wire.Send(contracts.IPCFrame{T: "Prepared", CommandID: frame.CommandID, Ticket: key, NativeState: before, Route: "sim-reminders", Fingerprint: frame.RequestHash, CandidateIDs: []string{a.NativeKey}})
		return err
	case "Dispatch":
		f.mu.Lock()
		t, ok := f.tickets[frame.Ticket]
		delete(f.tickets, frame.Ticket)
		if !ok || t.CommandID != frame.CommandID || t.Fence != fence || frame.OwnerFence != fence || !time.Now().Before(t.Expires) {
			f.mu.Unlock()
			return reject(contracts.ErrExpired)
		}
		if t.Action.NativeKey != "" && string(f.records[t.Action.NativeKey].Data) != string(t.Before) {
			f.mu.Unlock()
			return reject(contracts.ErrConflict)
		}
		if t.Action.Op != "reminders.create_list" {
			var list struct {
				Writable bool `json:"writable"`
			}
			json.Unmarshal(f.records[t.Action.ListKey].Data, &list)
			if !list.Writable {
				f.mu.Unlock()
				return reject(contracts.ErrUnsupported)
			}
		}
		if err := f.guard(guardEntry{CommandID: t.CommandID, Ticket: frame.Ticket, StartedAt: time.Now().UTC()}); err != nil {
			f.mu.Unlock()
			return reject(contracts.ErrUnavailable)
		}
		a := t.Action
		native := a.NativeKey
		if native == "" {
			native = contracts.NewID("native")
		}
		collection := "reminders.reminder"
		if a.Op == "reminders.create_list" {
			collection = "reminders.list"
		}
		var data map[string]any
		if len(t.Before) > 0 {
			decodeNative(t.Before, &data)
		} else {
			data = map[string]any{}
		}
		encoded, _ := json.Marshal(a.Args)
		var patch map[string]any
		json.Unmarshal(encoded, &patch)
		delete(patch, "id")
		delete(patch, "list_id")
		for k, v := range patch {
			data[k] = v
		}
		if collection == "reminders.list" {
			data["writable"] = true
		} else {
			if _, ok := data["completed"]; !ok {
				data["completed"] = false
			}
			data["list_id"] = map[string]any{"$ref": map[string]string{"collection": "reminders.list", "native_key": a.ListKey}}
		}
		if a.Op == "reminders.complete" {
			data["completed"] = true
		}
		if a.Op == "reminders.uncomplete" {
			data["completed"] = false
		}
		raw, _ := json.Marshal(data)
		o := contracts.Observation{Kind: "upsert", Collection: collection, NativeKey: native, Data: raw, ObservedAt: time.Now().UTC()}
		if a.Op == "reminders.delete" {
			o.Kind = "absent"
			o.Revalidated = true
		}
		f.records[native] = o
		f.effects[t.CommandID]++
		evidence, _ := json.Marshal(map[string]any{"native_key": native, "fingerprint": t.Hash, "observed": true, "absent": o.Kind == "absent"})
		outcome := contracts.IPCFrame{T: "Outcome", CommandID: t.CommandID, Disposition: "locally_observed", Evidence: evidence}
		f.evidence[t.CommandID] = outcome
		fault := f.fault
		f.fault = ""
		if err := f.saveLocked(); err != nil {
			f.mu.Unlock()
			return err
		}
		f.mu.Unlock()
		if fault == "crash-after-effect" || fault == "crash-before-outcome" {
			os.Exit(78)
		}
		capture := contracts.Capture{Source: contracts.Reminders, Observations: []contracts.Observation{o}}
		if o.Kind == "absent" {
			capture = fullScan([]contracts.Observation{o})
		}
		if err := emit(capture); err != nil {
			return err
		}
		if fault == "ambiguous-outcome" {
			outcome.Disposition = "indeterminate"
		}
		if err := f.guard(guardEntry{CommandID: t.CommandID, Outcome: &outcome}); err != nil {
			return err
		}
		_, err := wire.Send(outcome)
		return err
	case "Resolve":
		f.mu.Lock()
		outcome, ok := f.evidence[frame.CommandID]
		if ok {
			var evidence struct {
				NativeKey string `json:"native_key"`
				Absent    bool   `json:"absent"`
			}
			if json.Unmarshal(outcome.Evidence, &evidence) != nil {
				ok = false
			} else {
				record, exists := f.records[evidence.NativeKey]
				ok = exists && ((record.Kind == "absent") == evidence.Absent)
			}
		}
		f.mu.Unlock()
		if !ok {
			outcome = contracts.IPCFrame{T: "Outcome", CommandID: frame.CommandID, Disposition: "indeterminate"}
		}
		_, err := wire.Send(outcome)
		return err
	}
	return contracts.ErrInvalid
}

func decodeNative(raw []byte, out any) error {
	d := json.NewDecoder(bytes.NewReader(raw))
	d.UseNumber()
	return d.Decode(out)
}
func (f *fakeApple) loadGuards() error {
	file, err := os.Open(filepath.Join(f.home, "dispatch-guard.log"))
	if os.IsNotExist(err) {
		return nil
	}
	if err != nil {
		return err
	}
	defer file.Close()
	started := map[string]bool{}
	scanner := bufio.NewScanner(file)
	scanner.Buffer(make([]byte, 4096), contracts.MaxFrame)
	for scanner.Scan() {
		var entry guardEntry
		if json.Unmarshal(scanner.Bytes(), &entry) != nil {
			return contracts.ErrCorrupt
		}
		if entry.Outcome == nil {
			started[entry.CommandID] = true
		} else {
			delete(started, entry.CommandID)
		}
	}
	if scanner.Err() != nil {
		return scanner.Err()
	}
	for id := range started {
		f.recovered = append(f.recovered, contracts.IPCFrame{T: "Outcome", CommandID: id, Disposition: "indeterminate"})
	}
	return nil
}
