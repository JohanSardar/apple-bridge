package web

import (
	"context"
	"encoding/json"
	"testing"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/operations"
	"github.com/JohanSardar/apple-bridge/internal/state"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

type resolveReadTarget struct {
	status func(context.Context, contracts.CommandQuery) (contracts.CommandRecord, error)
}

func (r resolveReadTarget) Submit(context.Context, trust.Principal, contracts.Command) (contracts.CommandRecord, error) {
	panic("resolve must not submit")
}
func (r resolveReadTarget) List(context.Context, trust.Principal, contracts.CommandQuery) ([]contracts.CommandRecord, error) {
	panic("resolve must not list")
}
func (r resolveReadTarget) Status(ctx context.Context, _ trust.Principal, q contracts.CommandQuery) (contracts.CommandRecord, error) {
	return r.status(ctx, q)
}

func TestResolveInitialStatusBoundAndReplacementSnapshot(t *testing.T) {
	for _, blocked := range []bool{false, true} {
		name := "error"
		if blocked {
			name = "blocked"
		}
		t.Run(name, func(t *testing.T) {
			store, err := state.Open(t.TempDir())
			if err != nil {
				t.Fatal(err)
			}
			t.Cleanup(func() { store.Close() })
			id, chat := contracts.NewCommandID(), contracts.NewID("cht")
			head := store.Head()
			_, err = store.ApplyBatch(context.Background(), contracts.Batch{
				Epoch: head.Epoch, SubscriptionRevision: head.SubscriptionRevision, After: head.Sequence, Through: head.Sequence + 2,
				Changes: []contracts.Change{
					{Sequence: head.Sequence + 1, Op: "upsert", Record: contracts.Record{Collection: "system.command", ID: id, Version: 1, Source: contracts.Messages, Data: json.RawMessage(`{"principal":"test-principal"}`)}},
					{Sequence: head.Sequence + 2, Op: "upsert", Record: contracts.Record{Collection: "messages.chat", ID: chat, Version: 1, Source: contracts.Messages, Data: json.RawMessage(`{"display_name":"[ab-test] Resolve","participants":[],"service":"iMessage"}`)}},
				},
			})
			if err != nil {
				t.Fatal(err)
			}
			called := make(chan context.Context, 1)
			target := resolveReadTarget{status: func(ctx context.Context, q contracts.CommandQuery) (contracts.CommandRecord, error) {
				if q.ID != id || q.Resolve {
					t.Errorf("initial status query: %+v", q)
				}
				called <- ctx
				if blocked {
					<-ctx.Done()
					return contracts.CommandRecord{}, ctx.Err()
				}
				return contracts.CommandRecord{}, contracts.ErrUnavailable
			}}
			socket := &liveSocket{ctx: context.Background(), queue: make(chan liveFrame, 8), server: &server{deps: Deps{Operations: &operations.Service{State: store, Target: target}}}, identity: identity{Principal: trust.Principal{ID: "test-principal", Kind: "device", Grants: []contracts.Grant{"messages:read"}}}}
			done := make(chan struct{})
			started := time.Now()
			go func() { socket.resolveCommand(id); close(done) }()
			var statusContext context.Context
			select {
			case statusContext = <-called:
			case <-time.After(5 * time.Second):
				t.Fatal("initial status was not reached")
			}
			deadline, ok := statusContext.Deadline()
			if !ok || deadline.Before(started.Add(14*time.Second)) || deadline.After(started.Add(16*time.Second)) {
				t.Fatalf("missing 15-second resolve deadline: %v %v", deadline, ok)
			}
			select {
			case <-done:
			case <-time.After(17 * time.Second):
				t.Fatal("resolve did not finish within its deadline")
			}
			if blocked && statusContext.Err() != context.DeadlineExceeded {
				t.Fatalf("blocked read: %v", statusContext.Err())
			}
			var response struct {
				T    string `json:"t"`
				ID   string `json:"id"`
				Code string `json:"code"`
			}
			select {
			case frame := <-socket.queue:
				if err = json.Unmarshal(frame.Data, &response); err != nil {
					t.Fatal(err)
				}
			default:
				t.Fatal("resolve went silent")
			}
			if response.T != "error" || response.ID != id || response.Code != "unavailable" {
				t.Fatalf("uncorrelated error: %+v", response)
			}
			select {
			case extra := <-socket.queue:
				t.Fatalf("duplicate response: %s", extra.Data)
			default:
			}
			ctx, cancel := context.WithCancel(context.Background())
			defer cancel()
			args, _ := json.Marshal(map[string]any{"chat_id": chat, "anchor": "latest", "limit": 100})
			snapshotDone := make(chan struct{})
			go func() {
				socket.subscribe(ctx, clientFrame{ID: "replacement-thread", View: "thread", Args: args})
				close(snapshotDone)
			}()
			select {
			case frame := <-socket.queue:
				var snapshot struct {
					T    string `json:"t"`
					Sub  string `json:"sub"`
					Data struct {
						Items []json.RawMessage `json:"items"`
					} `json:"data"`
					Cursor contracts.Cursor `json:"cursor"`
				}
				if err = json.Unmarshal(frame.Data, &snapshot); err != nil {
					t.Fatal(err)
				}
				if snapshot.T != "snap" || snapshot.Sub != "replacement-thread" || snapshot.Data.Items == nil || snapshot.Cursor != store.Head() {
					t.Fatalf("replacement thread did not receive a fresh snapshot: %s", frame.Data)
				}
			case <-time.After(5 * time.Second):
				t.Fatal("replacement thread stayed loading after resolve")
			}
			cancel()
			select {
			case <-snapshotDone:
			case <-time.After(5 * time.Second):
				t.Fatal("replacement subscription did not stop")
			}
		})
	}
}
