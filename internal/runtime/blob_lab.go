//go:build lab

package runtime

import (
	"bytes"
	"context"
	"encoding/json"
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
