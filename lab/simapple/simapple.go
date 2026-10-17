package simapple

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"net"
	"net/http"
	"os"
	"path/filepath"
	"slices"
	"sync"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/applepeer"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

type Config struct {
	Socket     string
	SecretFile string
	Control    string
}
type Action struct {
	OmitCapture bool            `json:"omit_capture"`
	Count       int             `json:"count"`
	Key         string          `json:"key"`
	Collection  string          `json:"collection"`
	Data        json.RawMessage `json:"data"`
	Delete      bool            `json:"delete"`
}
type captureRequest struct {
	SourceState *contracts.IPCFrame
	Capture     contracts.Capture
	Done        chan struct{}
	Revision    uint64
	Close       bool
}
type fakeApple struct {
	contactPermissionMissing     bool
	contactSequence              uint64
	messages                     messageState
	liveChanges                  chan captureRequest
	permissionMissing            bool
	blobs                        map[string][]byte
	blobState                    string
	blobDelay                    time.Duration
	generation                   string
	recovered                    []contracts.IPCFrame
	home                         string
	effects                      map[string]int
	evidence                     map[string]contracts.IPCFrame
	tickets                      map[string]ticket
	fault                        string
	pause                        time.Duration
	blobDrops                    int
	blobBusy                     int
	blobFaultDigest              string
	blobRefreshBeforeUnavailable bool
	calendarDenied               bool
	calendarClock                time.Time

	completed map[contracts.Source]map[uint64]bool
	path      string
	durable   durableApple
	mu        sync.Mutex
	records   map[string]contracts.Observation
	changes   chan captureRequest
}

func (f *fakeApple) inspect() []contracts.Observation {
	f.mu.Lock()
	defer f.mu.Unlock()
	records := []contracts.Observation{}
	for _, o := range f.records {
		if !f.calendarInWindow(o) {
			continue
		}
		records = append(records, o)
	}
	slices.SortFunc(records, func(a, b contracts.Observation) int {
		if a.NativeKey < b.NativeKey {
			return -1
		}
		if a.NativeKey > b.NativeKey {
			return 1
		}
		return 0
	})
	return records
}
func (f *fakeApple) enqueue(ctx context.Context, c contracts.Capture, wait bool, revisions ...uint64) error {
	revision := uint64(0)
	if len(revisions) > 0 {
		revision = revisions[0]
	}
	req := captureRequest{Capture: c, Revision: revision}
	if wait {
		req.Done = make(chan struct{})
	}
	target := f.changes
	if c.Source == contracts.Messages && c.ScanID == "" {
		target = f.liveChanges
	}
	select {
	case target <- req:
	case <-ctx.Done():
		return ctx.Err()
	}
	if wait {
		select {
		case <-req.Done:
		case <-ctx.Done():
			return ctx.Err()
		}
	}
	return nil
}
func fullScan(records []contracts.Observation) contracts.Capture {
	return fullScanSource(records, contracts.Reminders)
}
func fullScanSource(records []contracts.Observation, source contracts.Source) contracts.Capture {
	observations := []contracts.Observation{}
	scope := []string{}
	for _, record := range records {
		if contracts.CollectionSource(record.Collection) == source {
			observations = append(observations, record)
			if !slices.Contains(scope, record.Collection) {
				scope = append(scope, record.Collection)
			}
		}
	}
	return contracts.Capture{Source: source, Observations: observations, ScanID: contracts.NewID("scan"), Scope: scope, ScanStartedAt: time.Now().UTC(), CompletionEvidence: "complete"}
}
func Run(ctx context.Context, cfg Config) error {
	fake := &fakeApple{blobs: map[string][]byte{}, generation: contracts.NewID("sim"), home: filepath.Dir(cfg.Control), effects: map[string]int{}, evidence: map[string]contracts.IPCFrame{}, tickets: map[string]ticket{}, records: map[string]contracts.Observation{}, changes: make(chan captureRequest, 4096), liveChanges: make(chan captureRequest, 4096)}
	fake.path = filepath.Join(filepath.Dir(cfg.Control), "sim-state.json")
	fake.durable = durableApple{Generation: contracts.NewID("sim"), Records: fake.records, Revision: map[contracts.Source]uint64{}, Committed: map[contracts.Source]uint64{}, Checkpoints: map[contracts.Source]string{}}
	if data, err := os.ReadFile(fake.path); err == nil {
		if err = json.Unmarshal(data, &fake.durable); err != nil {
			return err
		}
		fake.records = fake.durable.Records
		if fake.durable.Blobs != nil {
			fake.blobs = fake.durable.Blobs
		}
		fake.blobState = fake.durable.BlobState
		fake.blobDelay = time.Duration(fake.durable.BlobDelayNS)
	}
	if data, err := os.ReadFile(filepath.Join(filepath.Dir(fake.path), "sim-progress.json")); err == nil {
		var progress durableApple
		if json.Unmarshal(data, &progress) == nil && progress.Generation == fake.durable.Generation {
			fake.durable.Committed = progress.Committed
			fake.durable.Checkpoints = progress.Checkpoints
			fake.durable.Scans = progress.Scans
			fake.durable.InProgress = progress.InProgress
		}
	}
	if fake.durable.InProgress == nil {
		fake.durable.InProgress = map[contracts.Source]bool{}
	}
	fake.generation = fake.durable.Generation
	fake.contactSequence = fake.durable.ContactSequence
	fake.contactPermissionMissing = fake.durable.ContactPermissionMissing
	if fake.durable.Effects != nil {
		fake.effects = fake.durable.Effects
	}
	if fake.durable.Evidence != nil {
		fake.evidence = fake.durable.Evidence
	}
	fake.calendarClock = fake.durable.CalendarClock
	fake.calendarDenied = fake.durable.CalendarDenied
	if err := fake.loadGuards(); err != nil {
		return err
	}
	fake.messages = messageState{generation: fake.durable.MessagesGeneration, permitted: true}
	if fake.messages.generation == "" {
		fake.messages.generation = fake.durable.Generation
	}
	if err := fake.saveLocked(); err != nil {
		return err
	}
	mux := http.NewServeMux()
	fake.commandControl(mux)
	fake.blobRoutes(mux)
	fake.remindersControl(mux)
	fake.messageRoutes(mux)
	mux.HandleFunc("GET /stats", func(w http.ResponseWriter, r *http.Request) {
		fake.mu.Lock()
		defer fake.mu.Unlock()
		digests := map[string]string{}
		for digest, data := range fake.blobs {
			sum := sha256.Sum256(data)
			digests[digest] = hex.EncodeToString(sum[:])
		}
		json.NewEncoder(w).Encode(map[string]any{"blob_digests": digests, "scans": fake.durable.Scans, "revision": fake.durable.Revision, "committed": fake.durable.Committed})
	})
	fake.calendarControl(mux)
	fake.contactsControl(mux)
	mux.HandleFunc("GET /inspect", func(w http.ResponseWriter, r *http.Request) { json.NewEncoder(w).Encode(fake.inspect()) })
	mux.HandleFunc("POST /source-state", func(w http.ResponseWriter, r *http.Request) {
		var frame contracts.IPCFrame
		if json.NewDecoder(r.Body).Decode(&frame) != nil || (frame.Source != contracts.Reminders && frame.Source != contracts.Contacts && frame.Source != contracts.Calendar) || frame.State != "permission_missing" {
			http.Error(w, "invalid", 400)
			return
		}
		fake.mu.Lock()
		if frame.Source == contracts.Calendar {
			fake.calendarDenied = true
		} else if frame.Source == contracts.Contacts {
			fake.contactPermissionMissing = true
		} else {
			fake.permissionMissing = true
		}
		if err := fake.saveLocked(); err != nil {
			fake.mu.Unlock()
			http.Error(w, "persist", 500)
			return
		}
		fake.mu.Unlock()
		frame.T = "SourceState"
		frame.Coverage = map[contracts.Source]contracts.Coverage{frame.Source: {Fields: "partial", Missing: []string{"permission_missing"}}}
		select {
		case fake.changes <- captureRequest{SourceState: &frame}:
			json.NewEncoder(w).Encode(map[string]bool{"armed": true})
		case <-r.Context().Done():
		}
	})
	mux.HandleFunc("POST /seed", func(w http.ResponseWriter, r *http.Request) {
		var action Action
		if json.NewDecoder(http.MaxBytesReader(w, r.Body, 4096)).Decode(&action) != nil {
			http.Error(w, "invalid", 400)
			return
		}
		now := time.Now().UTC()
		fake.mu.Lock()
		fake.records["list"] = contracts.Observation{Kind: "upsert", Collection: "reminders.list", NativeKey: "list", ObservedAt: now, Data: json.RawMessage(`{"title":"Bridge Test","writable":true}`)}
		for i := 0; i < action.Count; i++ {
			key := contracts.NewID("seed")
			data, _ := json.Marshal(map[string]any{"title": "[ab-test] " + key, "completed": false, "opaque_number": json.Number("9007199254740993"), "list_id": map[string]any{"$ref": map[string]string{"collection": "reminders.list", "native_key": "list"}}})
			collection := "reminders.reminder"
			if action.Collection == "messages.message" {
				collection = action.Collection
				data, _ = json.Marshal(map[string]any{"title": "[ab-test] " + key, "text": fmt.Sprintf("[ab-test] synthetic message %d benchmark%s", i, map[bool]string{true: " needle"}[i%1000 == 0]), "optional_future": map[string]any{"counter": json.Number("9007199254740993")}})
			}
			fake.records[key] = contracts.Observation{Kind: "upsert", Collection: collection, NativeKey: key, ObservedAt: now, Data: data}
		}
		source := contracts.Reminders
		if action.Collection == "messages.message" {
			source = contracts.Messages
		}
		fake.changedLocked(source)
		revision := fake.durable.Revision[source]
		fake.mu.Unlock()
		capture := fullScanSource(fake.inspect(), source)
		if err := fake.enqueue(r.Context(), capture, false, revision); err != nil {
			return
		}
		json.NewEncoder(w).Encode(map[string]bool{"effect_done": true})
	})
	mux.HandleFunc("POST /external-change", func(w http.ResponseWriter, r *http.Request) {
		var action Action
		if json.NewDecoder(http.MaxBytesReader(w, r.Body, contracts.MaxFrame)).Decode(&action) != nil || action.Key == "" {
			http.Error(w, "invalid", 400)
			return
		}
		if action.Collection == "" {
			action.Collection = "reminders.reminder"
		}
		fake.mu.Lock()
		var o contracts.Observation
		if action.Delete {
			old, ok := fake.records[action.Key]
			if !ok {
				if err := fake.saveLocked(); err != nil {
					fake.mu.Unlock()
					http.Error(w, "persist", 500)
					return
				}
				fake.mu.Unlock()
				http.Error(w, "not found", 404)
				return
			}
			o = old
			o.Kind = "absent"
			o.ObservedAt = time.Now().UTC()
			o.Revalidated = true
		} else {
			if len(action.Data) == 0 {
				action.Data, _ = json.Marshal(map[string]any{"title": "[ab-test] " + action.Key, "completed": false, "opaque_number": json.Number("9007199254740993"), "list_id": map[string]any{"$ref": map[string]string{"collection": "reminders.list", "native_key": "list"}}})
			}
			o = contracts.Observation{Kind: "upsert", Collection: action.Collection, NativeKey: action.Key, ObservedAt: time.Now().UTC(), Data: action.Data}
		}
		if contracts.CollectionSource(o.Collection) == contracts.Contacts {
			o.ScopedAlias = "contacts/" + o.Collection + "/" + o.NativeKey
			fake.contactSequence++
		}
		fake.records[action.Key] = o
		if err := fake.changedLocked(contracts.CollectionSource(o.Collection)); err != nil {
			fake.mu.Unlock()
			http.Error(w, "persist", 500)
			return
		}
		permissionMissing := fake.permissionMissing
		revision := fake.durable.Revision[contracts.CollectionSource(o.Collection)]
		if contracts.CollectionSource(o.Collection) == contracts.Contacts {
			permissionMissing = fake.contactPermissionMissing
		}
		fake.mu.Unlock()
		if permissionMissing && (contracts.CollectionSource(o.Collection) == contracts.Reminders || contracts.CollectionSource(o.Collection) == contracts.Contacts) {
			json.NewEncoder(w).Encode(map[string]bool{"effect_done": true})
			return
		}
		capture := contracts.Capture{Source: contracts.CollectionSource(o.Collection), Observations: []contracts.Observation{o}}
		if action.Delete {
			capture = fullScanSource([]contracts.Observation{o}, contracts.CollectionSource(o.Collection))
		}
		if contracts.CollectionSource(o.Collection) == contracts.Contacts {
			capture = fake.contactSnapshot(false)
		}
		if !action.OmitCapture {
			if err := fake.enqueue(r.Context(), capture, false, revision); err != nil {
				return
			}
		}
		json.NewEncoder(w).Encode(map[string]bool{"effect_done": true})
	})
	mux.HandleFunc("POST /capture", func(w http.ResponseWriter, r *http.Request) {
		var capture contracts.Capture
		if json.NewDecoder(http.MaxBytesReader(w, r.Body, contracts.MaxFrame)).Decode(&capture) != nil {
			http.Error(w, "invalid", 400)
			return
		}
		if capture.Source == "" {
			capture.Source = contracts.Reminders
		}
		if fake.enqueue(r.Context(), capture, true) != nil {
			return
		}
		json.NewEncoder(w).Encode(map[string]bool{"committed": true})
	})
	mux.HandleFunc("POST /flush", func(w http.ResponseWriter, r *http.Request) {
		if fake.enqueue(r.Context(), contracts.Capture{Source: contracts.Messages}, true) != nil {
			return
		}
		if fake.enqueue(r.Context(), contracts.Capture{Source: contracts.Reminders}, true) != nil {
			return
		}
		json.NewEncoder(w).Encode(map[string]bool{"committed": true})
	})
	mux.HandleFunc("POST /close", func(w http.ResponseWriter, r *http.Request) {
		select {
		case fake.changes <- captureRequest{Close: true}:
		case <-r.Context().Done():
			return
		}
		json.NewEncoder(w).Encode(map[string]bool{"closed": true})
	})
	if err := os.MkdirAll(filepath.Dir(cfg.Control), 0700); err != nil {
		return err
	}
	os.Remove(cfg.Control)
	listener, err := net.Listen("unix", cfg.Control)
	if err != nil {
		return err
	}
	defer listener.Close()
	defer os.Remove(cfg.Control)
	if err = os.Chmod(cfg.Control, 0600); err != nil {
		return err
	}
	server := &http.Server{Handler: mux, ReadHeaderTimeout: 5 * time.Second}
	defer server.Close()
	go server.Serve(listener)
	secret, err := os.ReadFile(cfg.SecretFile)
	if err != nil {
		return err
	}
	generation := fake.durable.Generation
	for ctx.Err() == nil {
		conn, err := net.DialUnix("unix", nil, &net.UnixAddr{Name: cfg.Socket, Net: "unix"})
		if err == nil {
			err = fake.session(ctx, conn, secret, generation)
			conn.Close()
			if err == errClosed {
				<-ctx.Done()
				return nil
			}
		}
		timer := time.NewTimer(100 * time.Millisecond)
		select {
		case <-ctx.Done():
			timer.Stop()
			return nil
		case <-timer.C:
		}
	}
	return nil
}

var errClosed = contracts.ErrUnavailable

func (f *fakeApple) session(ctx context.Context, conn *net.UnixConn, secret []byte, generation string) error {
	wire := &applepeer.Wire{Conn: conn}
	if err := applepeer.Authenticate(wire, secret, false); err != nil {
		return err
	}
	open, _, err := wire.Receive()
	if err != nil {
		return err
	}
	if open.T != "Open" || open.Protocol != contracts.IPCProtocol || open.OwnerFence == 0 || open.ByteCredit <= 0 {
		return contracts.ErrInvalid
	}
	f.mu.Lock()
	f.tickets = map[string]ticket{}
	f.mu.Unlock()
	credit := open.ByteCredit
	coverage := contracts.Coverage{Fields: "complete", Missing: []string{"recurring reminders: next incomplete instance only"}}
	if _, err = wire.Send(contracts.IPCFrame{T: "Ready", Protocol: contracts.IPCProtocol, Capabilities: map[contracts.Source][]string{contracts.Notes: {"notes.create", "notes.update", "notes.delete", "notes.create_folder"}, contracts.Calendar: {"calendar.create_event", "calendar.update_event", "calendar.delete_event", "calendar.create_calendar"}, contracts.Contacts: {"contacts.create", "contacts.update", "contacts.delete"}, contracts.Messages: messageCapabilities(), contracts.Reminders: {"reminders.create", "reminders.update", "reminders.complete", "reminders.uncomplete", "reminders.delete", "reminders.create_list"}}, Coverage: map[contracts.Source]contracts.Coverage{contracts.Notes: {Fields: "complete", Missing: []string{}}, contracts.Calendar: {Fields: "complete", Missing: []string{}}, contracts.Contacts: {Fields: "partial", Missing: []string{"note"}}, contracts.Reminders: coverage, contracts.Messages: messageCoverage()}, HelperBuild: "sim/1", OSBuild: "sim"}); err != nil {
		return err
	}
	f.mu.Lock()
	denied := f.calendarDenied
	f.mu.Unlock()
	if denied {
		if _, err = wire.Send(calendarPermissionFrame()); err != nil {
			return err
		}
	}
	checkpoints := map[contracts.Source]string{}
	generations := map[contracts.Source]string{}
	for source, cp := range open.Checkpoints {
		generations[source] = cp.Generation
		expectedGeneration := generation
		if source == contracts.Messages {
			f.mu.Lock()
			expectedGeneration = f.messages.generation
			f.mu.Unlock()
		}
		if cp.Generation == expectedGeneration || source == contracts.Contacts {
			checkpoints[source] = cp.Checkpoint
		}
	}
	ctx, cancel := context.WithCancel(ctx)
	defer cancel()
	go func() { <-ctx.Done(); conn.Close() }()
	received := make(chan contracts.IPCFrame, 8)
	go func() {
		defer cancel()
		for {
			reply, _, err := wire.Receive()
			if err != nil {
				return
			}
			select {
			case received <- reply:
			case <-ctx.Done():
				return
			}
		}
	}()
	pending := []contracts.IPCFrame{}
	var emitRevision uint64
	var emitFull bool
	scanActive := map[contracts.Source]bool{}
	transmit := func(capture contracts.Capture, final bool) error {
		capture = messageAliases(capture)
		if capture.Generation == "" {
			capture.Generation = generation
		}
		if capture.Source == contracts.Messages {
			f.mu.Lock()
			expected := f.messages.generation
			f.mu.Unlock()
			if capture.Generation != expected {
				capture.Generation = expected
			}
		}
		if capture.Source == contracts.Contacts && capture.Generation == generation {
			capture.Generation = f.contactGeneration()
		}
		if generations[capture.Source] != capture.Generation {
			checkpoints[capture.Source] = ""
		}
		for d := range capture.BlobHandles {
			capture.BlobHandles[d] = fmt.Sprintf("%d:%s", open.OwnerFence, d)
		}
		capture.Predecessor = checkpoints[capture.Source]
		if generations[capture.Source] != capture.Generation {
			capture.Predecessor = ""
		}
		if capture.Checkpoint == "" || capture.Source != contracts.Contacts {
			capture.Checkpoint = contracts.NewID("cp")
		}
		encoded, err := contracts.EncodeFrame(contracts.IPCFrame{T: "Capture", Capture: &capture})
		if err != nil {
			return err
		}
		if len(encoded) > credit {
			if capture.Source == contracts.Messages {
				f.mu.Lock()
				f.messages.creditWaits++
				f.mu.Unlock()
			}
			return contracts.ErrCapacity
		}
		captureStarted := time.Now()
		n, err := wire.Send(contracts.IPCFrame{T: "Capture", Capture: &capture})
		if err != nil {
			return err
		}
		credit -= n
		for {
			select {
			case <-ctx.Done():
				return ctx.Err()
			case reply := <-received:
				switch reply.T {
				case "BlobRead":
					go func() {
						if f.blobReply(ctx, wire, reply, open.OwnerFence) != nil {
							conn.Close()
						}
					}()
				case "Prepare", "Dispatch", "Resolve":
					pending = append(pending, reply)
				case "Committed":
					if reply.Source != capture.Source || reply.Checkpoint != capture.Checkpoint || reply.Credit != n {
						return contracts.ErrInvalid
					}
					checkpoints[capture.Source] = capture.Checkpoint
					generations[capture.Source] = capture.Generation
					credit += reply.Credit
					if capture.Source == contracts.Messages {
						f.mu.Lock()
						f.messages.captureCount++
						f.messages.captureAckMS += float64(time.Since(captureStarted)) / float64(time.Millisecond)
						f.messages.captureBytes += n
						f.mu.Unlock()
					}
					f.mu.Lock()
					if final {
						f.committedLocked(capture.Source, emitRevision, emitFull)
						f.durable.InProgress[capture.Source] = scanActive[capture.Source]
					}
					f.durable.Checkpoints[capture.Source] = capture.Checkpoint
					f.progressLocked()
					f.mu.Unlock()
					return nil
				case "Ping":
					if _, err = wire.Send(contracts.IPCFrame{T: "Pong"}); err != nil {
						return err
					}
				case "Pong":
				default:
					return contracts.ErrReset
				}
			}
		}
	}
	emit := func(capture contracts.Capture) error {
		if capture.Source == contracts.Calendar {
			f.mu.Lock()
			denied := f.calendarDenied
			f.mu.Unlock()
			if denied {
				_, err := wire.Send(calendarPermissionFrame())
				return err
			}
		}
		scanActive[capture.Source] = capture.ScanID != ""
		f.mu.Lock()
		f.durable.InProgress[capture.Source] = true
		err := f.progressLocked()
		f.mu.Unlock()
		if err != nil {
			return err
		}
		if len(capture.Observations) == 0 {
			scanActive[capture.Source] = false
			return transmit(capture, true)
		}
		slices.SortStableFunc(capture.Observations, func(a, b contracts.Observation) int {
			if a.Kind == "absent" && b.Kind != "absent" {
				return 1
			}
			if b.Kind == "absent" && a.Kind != "absent" {
				return -1
			}
			if a.Collection == "reminders.list" && b.Collection != "reminders.list" {
				return -1
			}
			if b.Collection == "reminders.list" && a.Collection != "reminders.list" {
				return 1
			}
			return 0
		})
		for offset := 0; offset < len(capture.Observations); {
			end := min(offset+64, len(capture.Observations))
			part := capture
			for {
				part = capture
				part.Observations = capture.Observations[offset:end]
				if part.Generation == "" {
					part.Generation = generation
				}
				part.Predecessor = checkpoints[part.Source]
				if part.Source != contracts.Contacts || end < len(capture.Observations) {
					part.Checkpoint = contracts.NewID("cp")
				}
				if end < len(capture.Observations) && part.Observations[len(part.Observations)-1].Kind != "absent" {
					part.CompletionEvidence = ""
				}
				if _, err := contracts.EncodeFrame(contracts.IPCFrame{T: "Capture", Capture: &part}); err == nil {
					break
				} else if err != contracts.ErrCapacity || end == offset+1 {
					return err
				}
				end--
			}

			if capture.ScanID != "" {
				for {
					select {
					case live := <-f.liveChanges:
						if live.SourceState != nil {
							if _, err := wire.Send(*live.SourceState); err != nil {
								return err
							}
							continue
						}
						revision, full := emitRevision, emitFull
						emitRevision, emitFull = live.Revision, false
						err := transmit(f.revalidateMessages(live.Capture), true)
						emitRevision, emitFull = revision, full
						if err != nil {
							return err
						}
						if live.Done != nil {
							close(live.Done)
						}
					default:
						f.mu.Lock()
						blocked := false
						if part.Source == contracts.Messages && f.messages.bulkPaused {
							for _, observation := range part.Observations {
								if observation.NativeKey == f.messages.bulkTarget {
									blocked = true
								}
							}
						}
						f.messages.bulkBlocked = blocked
						f.mu.Unlock()
						if blocked {
							select {
							case <-ctx.Done():
								return ctx.Err()
							case <-time.After(10 * time.Millisecond):
							}
							continue
						}
						goto drainedLive
					}
				}
			}
		drainedLive:
			if end == len(capture.Observations) {
				scanActive[capture.Source] = false
			}
			if err := transmit(f.revalidateMessages(part), end == len(capture.Observations)); err != nil {
				return err
			}
			offset = end
		}
		return nil
	}
	commandEmit := func(capture contracts.Capture, revision uint64) error {
		if capture.Source == contracts.Calendar {
			f.mu.Lock()
			denied := f.calendarDenied
			f.mu.Unlock()
			if denied {
				return contracts.ErrUnavailable
			}
		}
		emitFull = false
		emitRevision = revision
		return emit(capture)
	}
	for _, source := range []contracts.Source{contracts.Reminders, contracts.Messages, contracts.Contacts, contracts.Calendar, contracts.Notes} {
		f.mu.Lock()
		missingContacts := f.contactPermissionMissing
		f.mu.Unlock()
		if source == contracts.Contacts && missingContacts {
			if _, err := wire.Send(contracts.IPCFrame{T: "SourceState", Source: contracts.Contacts, State: "permission_missing", Coverage: map[contracts.Source]contracts.Coverage{contracts.Contacts: {Fields: "partial", Missing: []string{"permission_missing"}}}}); err != nil {
				return err
			}
			continue
		}
		f.mu.Lock()
		emitRevision = f.durable.Revision[source]
		scan := (source == contracts.Messages && open.Checkpoints[source].RefreshBlobs) || f.durable.InProgress[source] || checkpoints[source] == "" || checkpoints[source] != f.durable.Checkpoints[source] || emitRevision != f.durable.Committed[source]
		if scan {
			f.durable.Scans++
			f.progressLocked()
		}
		f.mu.Unlock()
		if scan {
			capture := fullScanSource(f.inspect(), source)
			if source == contracts.Contacts {
				capture = f.contactSnapshot(false)
			} else if source == contracts.Calendar {
				f.mu.Lock()
				capture = f.calendarScanLocked()
				emitRevision = f.durable.Revision[source]
				f.mu.Unlock()
			}
			if source == contracts.Messages {
				capture.BlobHandles = f.blobHandles()
			}
			emitFull = true
			err = emit(capture)
			emitFull = false
			if err != nil {
				return err
			}
		}
	}
	for _, outcome := range f.recovered {
		if _, err = wire.Send(outcome); err != nil {
			return err
		}
	}
	f.recovered = nil
	for {
		if len(pending) > 0 {
			reply := pending[0]
			pending = pending[1:]
			if err = f.command(wire, reply, open.OwnerFence, commandEmit, func(source contracts.Source) string { return checkpoints[source] }); err != nil {
				return err
			}
			continue
		}
		select {
		case <-ctx.Done():
			return ctx.Err()
		case reply := <-received:
			if reply.T == "Prepare" || reply.T == "Dispatch" || reply.T == "Resolve" {
				if err = f.command(wire, reply, open.OwnerFence, commandEmit, func(source contracts.Source) string { return checkpoints[source] }); err != nil {
					return err
				}
				continue
			}
			if reply.T == "BlobRead" {
				go func() {
					if f.blobReply(ctx, wire, reply, open.OwnerFence) != nil {
						conn.Close()
					}
				}()
			} else if reply.T == "Ping" {
				if _, err = wire.Send(contracts.IPCFrame{T: "Pong"}); err != nil {
					return err
				}
			} else if reply.T != "Pong" {
				return contracts.ErrInvalid
			}
		case req := <-f.liveChanges:
			emitRevision, emitFull = req.Revision, false
			if req.SourceState != nil {
				if _, err = wire.Send(*req.SourceState); err != nil {
					return err
				}
				continue
			}
			if err = emit(req.Capture); err != nil {
				return err
			}
			if req.Done != nil {
				close(req.Done)
			}
		case req := <-f.changes:
			if req.SourceState != nil {
				if _, err := wire.Send(*req.SourceState); err != nil {
					return err
				}
				continue
			}
			if req.Close {
				wire.Send(contracts.IPCFrame{T: "Close", Reason: "lab"})
				return errClosed
			}
			emitRevision = req.Revision
			emitFull = req.Capture.Source == contracts.Messages && req.Capture.ScanID != "" && req.Capture.CompletionEvidence == "complete"
			if err = emit(req.Capture); err != nil {
				return err
			}
			if req.Done != nil {
				close(req.Done)
			}
		}
	}
}
