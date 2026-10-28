package applepeer

import (
	"context"
	"encoding/json"
	"errors"
	"net"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func TestQueryBlockedWrites(t *testing.T) {
	for _, phase := range []string{"initial", "cancel", "queued"} {
		t.Run(phase, func(t *testing.T) {
			listener, err := net.ListenUnix("unix", &net.UnixAddr{Name: filepath.Join(t.TempDir(), "peer"), Net: "unix"})
			if err != nil {
				t.Fatal(err)
			}
			defer listener.Close()
			helper, err := net.DialUnix("unix", nil, listener.Addr().(*net.UnixAddr))
			if err != nil {
				t.Fatal(err)
			}
			defer helper.Close()
			conn, err := listener.AcceptUnix()
			if err != nil {
				t.Fatal(err)
			}
			defer conn.Close()
			conn.SetWriteBuffer(1024)
			wire := &Wire{Conn: conn}
			commands := &Commands{}
			commands.attach(wire, 1)
			args := json.RawMessage(`{"value":"` + strings.Repeat("x", 200<<10) + `"}`)
			var release chan struct{}
			if phase == "cancel" {
				release = make(chan struct{})
				go func() {
					var frame contracts.IPCFrame
					contracts.ReadFrame(helper, &frame)
					for i := 0; i < 1000; i++ {
						if _, err := wire.Send(contracts.IPCFrame{T: "Ping"}); err != nil {
							break
						}
					}
					close(release)
				}()
			}
			if phase == "queued" {
				wire.writeOnce.Do(func() { wire.writeSlot = make(chan struct{}, 1) })
				wire.writeSlot <- struct{}{}
				defer func() { <-wire.writeSlot }()
			}
			ctx, cancel := context.WithTimeout(context.Background(), 100*time.Millisecond)
			defer cancel()
			start := time.Now()
			_, err = commands.Exchange(ctx, contracts.IPCFrame{T: "Query", ID: "qry_blocked", Op: "test.echo", Args: args, DeadlineMS: 100})
			elapsed := time.Since(start)
			if !errors.Is(err, context.DeadlineExceeded) || elapsed > 200*time.Millisecond {
				t.Fatalf("query returned %v after %s", err, elapsed)
			}
			if commands.Available() {
				t.Fatal("blocked session remained available")
			}
			if release != nil {
				select {
				case <-release:
				case <-time.After(time.Second):
					t.Fatal("writer leaked")
				}
			}
		})
	}
}
