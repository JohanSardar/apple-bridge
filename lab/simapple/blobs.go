package simapple

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"github.com/JohanSardar/apple-bridge/internal/applepeer"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"net/http"
	"strings"
	"time"
)

func (f *fakeApple) blobHandles() map[string]string {
	f.mu.Lock()
	defer f.mu.Unlock()
	out := map[string]string{}
	for d := range f.blobs {
		out[d] = d
	}
	return out
}
func (f *fakeApple) blobRoutes(mux *http.ServeMux) {
	mux.HandleFunc("POST /blob-history", func(w http.ResponseWriter, r *http.Request) {
		var a struct {
			Rows int `json:"rows"`
		}
		if json.NewDecoder(http.MaxBytesReader(w, r.Body, 4096)).Decode(&a) != nil || a.Rows < 1 || a.Rows > 20000 {
			http.Error(w, "invalid", 400)
			return
		}
		now := time.Now().UTC()
		observations := make([]contracts.Observation, a.Rows)
		f.mu.Lock()
		for i := range observations {
			key := fmt.Sprintf("blob-history-%05d", i)
			data, _ := json.Marshal(map[string]any{"title": "[ab-test] " + key, "completed": false, "list_id": map[string]any{"$ref": map[string]string{"collection": "reminders.list", "native_key": "list"}}})
			observations[i] = contracts.Observation{Kind: "upsert", Collection: "reminders.reminder", NativeKey: key, ObservedAt: now, Data: data}
			f.records[key] = observations[i]
		}
		err := f.changedLocked(contracts.Reminders)
		revision := f.durable.Revision[contracts.Reminders]
		f.mu.Unlock()
		if err != nil {
			http.Error(w, "persist", 500)
			return
		}
		start := time.Now()
		if err = f.enqueue(r.Context(), contracts.Capture{Source: contracts.Reminders, Observations: observations}, true, revision); err != nil {
			http.Error(w, "capture", 503)
			return
		}
		elapsed := time.Since(start)
		json.NewEncoder(w).Encode(map[string]float64{"commit_ms": float64(elapsed) / float64(time.Millisecond), "rows_per_second": float64(a.Rows) / elapsed.Seconds()})
	})

	mux.HandleFunc("POST /blob-fixture", func(w http.ResponseWriter, r *http.Request) {
		var a struct {
			RefreshBeforeUnavailable bool   `json:"refresh_before_unavailable"`
			Busy                     int    `json:"unavailable_reads"`
			Drops                    int    `json:"drop_reads"`
			Size                     int    `json:"size"`
			State                    string `json:"state"`
			SlowMS                   int    `json:"slow_ms"`
			Delete                   bool   `json:"delete"`
		}
		if json.NewDecoder(http.MaxBytesReader(w, r.Body, 4096)).Decode(&a) != nil || a.Size < 0 || a.Size > 32<<20 {
			http.Error(w, "invalid", 400)
			return
		}
		f.mu.Lock()
		f.blobRefreshBeforeUnavailable = a.RefreshBeforeUnavailable
		f.blobBusy = a.Busy
		f.blobDrops = a.Drops
		f.blobState = a.State
		f.blobDelay = time.Duration(a.SlowMS) * time.Millisecond
		if a.Delete {
			for k, o := range f.records {
				if contracts.CollectionSource(o.Collection) == contracts.Messages {
					o.Kind = "absent"
					o.Revalidated = true
					o.ObservedAt = time.Now().UTC()
					f.records[k] = o
				}
			}
		} else {
			b := make([]byte, a.Size)
			for i := range b {
				b[i] = byte((i*31 + i/251) % 256)
			}
			sum := sha256.Sum256(b)
			d := hex.EncodeToString(sum[:])
			f.blobs[d] = b
			state := a.State
			if state == "" {
				state = "present"
			}
			ref := map[string]any{"$ref": map[string]string{"collection": "messages.message", "native_key": "blob-message"}}
			for k, data := range map[string]any{
				"blob-message":    map[string]any{"title": "[ab-test] blob-message", "text": "[ab-test] attachment", "kind": "message", "decode_status": "decoded", "parts": []any{map[string]any{"index": 0, "kind": "attachment", "attachment_id": map[string]any{"$ref": map[string]string{"collection": "messages.attachment", "native_key": "blob-attachment"}}}}},
				"blob-attachment": map[string]any{"title": "[ab-test] blob-attachment", "message_id": ref, "filename": "fixture.bin", "mime_type": "application/octet-stream", "uti": "public.data", "size": len(b), "blob": map[string]any{"digest": d, "state": state}},
			} {
				collection := "messages.message"
				if k == "blob-attachment" {
					collection = "messages.attachment"
				}
				raw, _ := json.Marshal(data)
				f.records[k] = contracts.Observation{Kind: "upsert", Collection: collection, NativeKey: k, ObservedAt: time.Now().UTC(), Data: raw}
			}
		}
		observations := []contracts.Observation{}
		for _, o := range f.records {
			if contracts.CollectionSource(o.Collection) == contracts.Messages {
				observations = append(observations, o)
			}
		}
		handles := map[string]string{}
		for d := range f.blobs {
			handles[d] = d
		}
		if err := f.changedLocked(contracts.Messages); err != nil {
			f.mu.Unlock()
			http.Error(w, "unavailable", 503)
			return
		}
		revision := f.durable.Revision[contracts.Messages]
		f.mu.Unlock()
		capture := contracts.Capture{Source: contracts.Messages, Observations: observations, BlobHandles: handles, ScanID: contracts.NewID("scan"), Scope: []string{"messages.message", "messages.attachment"}, ScanStartedAt: time.Now().UTC(), CompletionEvidence: "complete"}
		if err := f.enqueue(r.Context(), capture, false, revision); err != nil {
			http.Error(w, "unavailable", 503)
			return
		}
		json.NewEncoder(w).Encode(map[string]bool{"effect_done": true})
	})
}
func (f *fakeApple) blobReply(ctx context.Context, wire *applepeer.Wire, read contracts.IPCFrame, fence uint64) error {
	f.mu.Lock()
	digest := strings.TrimPrefix(read.Handle, fmt.Sprintf("%d:", fence))
	b, ok := f.blobs[digest]
	refreshBeforeUnavailable := f.blobRefreshBeforeUnavailable
	busy := f.blobBusy > 0
	if busy {
		f.blobBusy--
	}
	drop := f.blobDrops > 0
	if drop {
		f.blobDrops--
	}
	state, delay := f.blobState, f.blobDelay
	f.mu.Unlock()
	if drop {
		return nil
	}
	if busy {
		if refreshBeforeUnavailable {
			if err := f.enqueue(ctx, contracts.Capture{Source: contracts.Messages, BlobHandles: map[string]string{digest: digest}}, true); err != nil {
				return err
			}
		}
		_, err := wire.Send(contracts.IPCFrame{T: "BlobUnavailable", Handle: read.Handle, Reason: "unavailable"})
		return err
	}
	if !ok || state == "offloaded" || state == "missing" {
		if state == "" {
			state = "missing"
		}
		_, err := wire.Send(contracts.IPCFrame{T: "BlobUnavailable", Handle: read.Handle, Reason: state})
		return err
	}
	if read.Offset < 0 || read.Offset > int64(len(b)) || read.Length <= 0 || read.Length > 128<<10 {
		return contracts.ErrInvalid
	}
	if delay > 0 {
		timer := time.NewTimer(delay)
		defer timer.Stop()
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-timer.C:
		}
	}
	end := min(read.Offset+read.Length, int64(len(b)))
	_, err := wire.Send(contracts.IPCFrame{T: "BlobChunk", Handle: read.Handle, Offset: read.Offset, Bytes: b[read.Offset:end], Digest: digest, EOF: end == int64(len(b))})
	return err
}
