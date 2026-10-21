//go:build lab

package runtime

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"github.com/JohanSardar/apple-bridge/internal/blobs"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/state"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"net"
	"net/http"
	"os"
	"path/filepath"
	"sync"
	"time"
)

func serveBlobLab(ctx context.Context, home string, s *state.Store, b *blobs.Store) error {
	dir := filepath.Join(home, "run")
	if err := os.MkdirAll(dir, 0700); err != nil {
		return err
	}
	path := filepath.Join(dir, "blobs-lab.sock")
	os.Remove(path)
	listener, err := net.Listen("unix", path)
	if err != nil {
		return err
	}
	if err = os.Chmod(path, 0600); err != nil {
		listener.Close()
		return err
	}
	var mu sync.Mutex
	snapshots := map[string]*state.FrozenSnapshot{}
	server := &http.Server{ReadHeaderTimeout: 5 * time.Second, Handler: http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		mu.Lock()
		defer mu.Unlock()
		var a struct {
			Action string `json:"action"`
			Pin    string `json:"pin"`
			Digest string `json:"digest"`
			Owner  string `json:"owner"`
		}
		if json.NewDecoder(http.MaxBytesReader(w, r.Body, 4096)).Decode(&a) != nil {
			http.Error(w, "invalid", 400)
			return
		}
		var result any = map[string]bool{"ok": true}
		var err error
		switch a.Action {
		case "relay-fixture":
			payload := bytes.Repeat([]byte("[ab-test] relay blob\n"), 8192)
			var digest, pin string
			digest, pin, err = b.Stage(r.Context(), bytes.NewReader(payload), blobs.Expect{})
			if err != nil {
				err = fmt.Errorf("stage fixture: %w", err)
			}
			if err == nil {
				defer b.Release(context.WithoutCancel(r.Context()), pin)
				data, _ := json.Marshal(map[string]any{"title": "[ab-test] relay blob", "blob": contracts.BlobReference{Digest: digest, Size: int64(len(payload)), State: "available"}})
				_, err = s.CommitCapture(r.Context(), contracts.Capture{Source: contracts.System, Generation: contracts.NewID("relay"), Checkpoint: "1", Observations: []contracts.Observation{{Kind: "upsert", Collection: "system.source", NativeKey: "relay-blob", Data: data, ObservedAt: time.Now().UTC()}}})
				if err != nil {
					err = fmt.Errorf("commit fixture: %w", err)
				}
				result = map[string]any{"digest": digest, "size": len(payload), "title": "[ab-test] relay blob"}
			}
			if err != nil {
				os.WriteFile(filepath.Join(dir, "relay-blob-error.log"), []byte(err.Error()), 0600)
			}
		case "stage":
			d, p, e := b.Stage(r.Context(), bytes.NewReader([]byte("[ab-test] pinned blob")), blobs.Expect{})
			err = e
			result = map[string]string{"digest": d, "pin": p}
		case "release":
			err = b.Release(r.Context(), a.Pin)
		case "snapshot":
			var snapshot *state.FrozenSnapshot
			snapshot, err = s.FreezeSnapshot(ctx, trust.AllVisibility(), 1)
			if err == nil {
				snapshots[a.Owner] = snapshot
			}
		case "snapshot-release":
			if snapshot := snapshots[a.Owner]; snapshot != nil {
				err = snapshot.Close()
				delete(snapshots, a.Owner)
			}
		case "command":
			err = s.SetBlobReferences(r.Context(), "command:"+a.Owner, []contracts.BlobReference{{Digest: a.Digest}})
		case "command-release":
			err = s.SetBlobReferences(r.Context(), "command:"+a.Owner, nil)
		default:
			http.Error(w, "invalid", 400)
			return
		}
		if err != nil {
			http.Error(w, "operation_failed", 500)
			return
		}
		json.NewEncoder(w).Encode(result)
	})}
	go server.Serve(listener)
	go func() { <-ctx.Done(); server.Close(); os.Remove(path) }()
	return nil
}
