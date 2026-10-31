package operations

import (
	"context"
	"encoding/json"
	"net"
	"os"
	"path/filepath"
	"testing"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/applepeer"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/state"
)

func TestMailCompletionRequiresMailCapture(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	store, err := state.Open(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	defer store.Close()
	directory, err := os.MkdirTemp("/tmp", "mail-completion-")
	if err != nil {
		t.Fatal(err)
	}
	defer os.RemoveAll(directory)
	socket := filepath.Join(directory, "peer")
	peer := &applepeer.Commands{}
	secret := []byte("synthetic-mail-completion-secret")
	ready := make(chan struct{}, 1)
	done := make(chan error, 1)
	go func() {
		done <- applepeer.Serve(ctx, applepeer.Config{
			Socket: socket, Secret: secret, Commands: peer,
			NextFence: func(context.Context) (uint64, error) { return 1, nil },
			Checkpoints: func(context.Context) (map[contracts.Source]contracts.Checkpoint, error) {
				return map[contracts.Source]contracts.Checkpoint{}, nil
			},
			SourceState: func(_ context.Context, _ contracts.Source, status string, _ contracts.Coverage, _ []string, _ string) error {
				if status == "ready" {
					ready <- struct{}{}
				}
				return nil
			},
		})
	}()
	defer func() {
		cancel()
		if err := <-done; err != nil {
			t.Error(err)
		}
	}()
	var connection *net.UnixConn
	for ctx.Err() == nil {
		connection, err = net.DialUnix("unix", nil, &net.UnixAddr{Name: socket, Net: "unix"})
		if err == nil {
			break
		}
		time.Sleep(time.Millisecond)
	}
	if err != nil {
		t.Fatal(err)
	}
	defer connection.Close()
	connection.SetDeadline(time.Now().Add(4 * time.Second))
	wire := &applepeer.Wire{Conn: connection}
	if err = applepeer.Authenticate(wire, secret, false); err != nil {
		t.Fatal(err)
	}
	if _, _, err = wire.Receive(); err != nil {
		t.Fatal(err)
	}
	if _, err = wire.Send(contracts.IPCFrame{T: "Ready", Protocol: contracts.IPCProtocol, Coverage: map[contracts.Source]contracts.Coverage{contracts.Mail: {Fields: "partial"}}}); err != nil {
		t.Fatal(err)
	}
	select {
	case <-ready:
	case <-ctx.Done():
		t.Fatal(ctx.Err())
	}
	gateway := LocalGateway{State: store, Peer: peer, Context: ctx}
	for _, test := range []struct {
		name        string
		source      contracts.Source
		collection  string
		fingerprint string
		observed    bool
	}{
		{"mail", contracts.Mail, "mail.message", "fingerprint", true},
		{"wrong-source", contracts.Reminders, "mail.message", "fingerprint", false},
		{"wrong-collection", contracts.Mail, "reminders.reminder", "fingerprint", false},
		{"wrong-command", contracts.Mail, "mail.message", "different", false},
	} {
		t.Run(test.name, func(t *testing.T) {
			peer.RecordCapture(contracts.Capture{Source: test.source, Checkpoint: test.name, Observations: []contracts.Observation{{Kind: "upsert", Collection: test.collection, NativeKey: "123"}}}, contracts.Cursor{Epoch: store.Head().Epoch, Sequence: 99})
			evidence, _ := json.Marshal(map[string]any{"observed": true, "native_key": "123", "fingerprint": test.fingerprint, "capture_checkpoint": test.name})
			result, err := gateway.outcome(ctx, contracts.CommandRecord{State: contracts.CommandObserved, Source: contracts.Mail, RequestHash: "fingerprint", Command: contracts.Command{Op: "mail.set_flag"}}, contracts.IPCFrame{T: "Outcome", Disposition: contracts.DispositionLocallyObserved, Evidence: evidence})
			if err != nil || (result.CommitSeq == 99) != test.observed {
				t.Fatalf("commit sequence=%d observed=%t err=%v", result.CommitSeq, test.observed, err)
			}
		})
	}
}
