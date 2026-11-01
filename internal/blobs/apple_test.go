package blobs

import (
	"context"
	"strings"
	"testing"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func TestCaptureEnqueuesWhileManifestIOIsBlocked(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	store := &Store{appleRunning: true, applePending: map[string]*appleJob{}, manifest: manifest{Entries: map[string]Entry{}}}
	digest := strings.Repeat("a", 64)
	prior := strings.Repeat("b", 64)
	store.applePending[prior] = &appleJob{ctx: ctx}
	store.manifest.Entries[prior] = Entry{State: "present"}
	store.mu.Lock()
	workerFinished := make(chan struct{})
	go func() { store.captureBlobs(); close(workerFinished) }()
	finished := make(chan struct{})
	go func() {
		store.Capture(ctx, contracts.Capture{Live: true, BlobHandles: map[string]string{digest: "handle"}}, nil)
		close(finished)
	}()
	select {
	case <-finished:
	case <-time.After(250 * time.Millisecond):
		cancel()
		store.mu.Unlock()
		<-finished
		t.Fatal("capture enqueue waited for unrelated manifest I/O")
	}
	cancel()
	store.mu.Unlock()
	store.appleMu.Lock()
	job := store.applePending[digest]
	store.appleMu.Unlock()
	if job == nil || !job.live || job.source.handle != "handle" {
		t.Fatal("live media work was not queued")
	}
	select {
	case <-workerFinished:
	case <-time.After(time.Second):
		t.Fatal("cancelled media worker did not finish")
	}
}
