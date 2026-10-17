package simapple

import (
	"context"
	"encoding/json"
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
	Capture contracts.Capture
	Done    chan struct{}
	Close   bool
}
type fakeApple struct {
	blobs      map[string][]byte
	blobState  string
	blobDelay  time.Duration
	generation string
	recovered  []contracts.IPCFrame
	home       string
	effects    map[string]int
	evidence   map[string]contracts.IPCFrame
	tickets    map[string]ticket
	fault      string
	pause      time.Duration

	mu      sync.Mutex
	records map[string]contracts.Observation
	changes chan captureRequest
}

func (f *fakeApple) inspect() []contracts.Observation {
	f.mu.Lock()
	defer f.mu.Unlock()
	records := []contracts.Observation{}
	for _, o := range f.records {
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
func (f *fakeApple) enqueue(ctx context.Context, c contracts.Capture, wait bool) error {
	req := captureRequest{Capture: c}
	if wait {
		req.Done = make(chan struct{})
	}
	select {
	case f.changes <- req:
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
	return contracts.Capture{Source: contracts.Reminders, Observations: records, ScanID: contracts.NewID("scan"), Scope: []string{"reminders.list", "reminders.reminder"}, ScanStartedAt: time.Now().UTC(), CompletionEvidence: "complete"}
}
func Run(ctx context.Context, cfg Config) error {
	fake := &fakeApple{blobs: map[string][]byte{}, generation: contracts.NewID("sim"), home: filepath.Dir(cfg.Control), effects: map[string]int{}, evidence: map[string]contracts.IPCFrame{}, tickets: map[string]ticket{}, records: map[string]contracts.Observation{}, changes: make(chan captureRequest, 4096)}
	var durable durableApple
	if b, err := os.ReadFile(filepath.Join(fake.home, "truth.json")); err == nil {
		if json.Unmarshal(b, &durable) != nil {
			return contracts.ErrCorrupt
		}
		if durable.Generation != "" {
			fake.generation = durable.Generation
		}
		fake.records = durable.Records
		fake.effects = durable.Effects
		fake.evidence = durable.Evidence
	}
	if err := fake.loadGuards(); err != nil {
		return err
	}
	mux := http.NewServeMux()
	fake.commandControl(mux)
	fake.blobRoutes(mux)
	mux.HandleFunc("GET /inspect", func(w http.ResponseWriter, r *http.Request) { json.NewEncoder(w).Encode(fake.inspect()) })
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
			fake.records[key] = contracts.Observation{Kind: "upsert", Collection: "reminders.reminder", NativeKey: key, ObservedAt: now, Data: data}
		}
		if err := fake.saveLocked(); err != nil {
			fake.mu.Unlock()
			http.Error(w, "persist", 500)
			return
		}
		fake.mu.Unlock()
		if err := fake.enqueue(r.Context(), fullScan(fake.inspect()), false); err != nil {
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
		fake.records[action.Key] = o
		if err := fake.saveLocked(); err != nil {
			fake.mu.Unlock()
			http.Error(w, "persist", 500)
			return
		}
		fake.mu.Unlock()
		capture := contracts.Capture{Source: contracts.CollectionSource(o.Collection), Observations: []contracts.Observation{o}}
		if action.Delete {
			capture = fullScan([]contracts.Observation{o})
		}
		if !action.OmitCapture {
			if err := fake.enqueue(r.Context(), capture, false); err != nil {
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
	generation := fake.generation
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
	coverage := contracts.Coverage{Fields: "complete", Missing: []string{}}
	if _, err = wire.Send(contracts.IPCFrame{T: "Ready", Protocol: contracts.IPCProtocol, Capabilities: map[contracts.Source][]string{contracts.Reminders: {"reminders.create", "reminders.update", "reminders.complete", "reminders.uncomplete", "reminders.delete", "reminders.create_list"}}, Coverage: map[contracts.Source]contracts.Coverage{contracts.Reminders: coverage, contracts.Messages: coverage}, HelperBuild: "sim/1", OSBuild: "sim"}); err != nil {
		return err
	}
	checkpoints := map[contracts.Source]string{}
	for source, cp := range open.Checkpoints {
		if cp.Generation == generation {
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
	transmit := func(capture contracts.Capture) error {
		capture.Generation = generation
		capture.Predecessor = checkpoints[capture.Source]
		capture.Checkpoint = contracts.NewID("cp")
		encoded, err := contracts.EncodeFrame(contracts.IPCFrame{T: "Capture", Capture: &capture})
		if err != nil {
			return err
		}
		if len(encoded) > credit {
			return contracts.ErrCapacity
		}
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
						if f.blobReply(ctx, wire, reply) != nil {
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
					credit += reply.Credit
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
		if len(capture.Observations) == 0 {
			return transmit(capture)
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
				part.Generation = generation
				part.Predecessor = checkpoints[part.Source]
				part.Checkpoint = contracts.NewID("cp")
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

			if err := transmit(part); err != nil {
				return err
			}
			offset = end
		}
		return nil
	}
	for _, source := range []contracts.Source{contracts.Reminders, contracts.Messages} {
		records := []contracts.Observation{}
		for _, o := range f.inspect() {
			if contracts.CollectionSource(o.Collection) == source {
				records = append(records, o)
			}
		}
		capture := fullScan(records)
		capture.Source = source
		if source == contracts.Messages {
			capture.Scope = []string{"messages.message", "messages.attachment"}
			capture.BlobHandles = f.blobHandles()
		}
		if err = emit(capture); err != nil {
			return err
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
			if err = f.command(wire, reply, open.OwnerFence, emit, func() string { return checkpoints[contracts.Reminders] }); err != nil {
				return err
			}
			continue
		}
		select {
		case <-ctx.Done():
			return ctx.Err()
		case reply := <-received:
			if reply.T == "Prepare" || reply.T == "Dispatch" || reply.T == "Resolve" {
				if err = f.command(wire, reply, open.OwnerFence, emit, func() string { return checkpoints[contracts.Reminders] }); err != nil {
					return err
				}
				continue
			}
			if reply.T == "BlobRead" {
				go func() {
					if f.blobReply(ctx, wire, reply) != nil {
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
		case req := <-f.changes:
			if req.Close {
				wire.Send(contracts.IPCFrame{T: "Close", Reason: "lab"})
				return errClosed
			}
			if err = emit(req.Capture); err != nil {
				return err
			}
			if req.Done != nil {
				close(req.Done)
			}
		}
	}
}
