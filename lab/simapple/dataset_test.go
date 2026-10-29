package simapple

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func TestDatasetStagesSourcesAfterPriorCapture(t *testing.T) {
	f := &fakeApple{
		path:        filepath.Join(t.TempDir(), "sim-state.json"),
		records:     map[string]contracts.Observation{},
		blobs:       map[string][]byte{},
		changes:     make(chan captureRequest),
		liveChanges: make(chan captureRequest),
		durable:     durableApple{Revision: map[contracts.Source]uint64{}},
	}
	mux := http.NewServeMux()
	f.datasetRoutes(mux)
	dataset := Dataset{Records: []contracts.Observation{
		{Kind: "upsert", Collection: "contacts.contact", NativeKey: "contact", Data: json.RawMessage(`{"given_name":"[ab-test] fixture"}`)},
		{Kind: "upsert", Collection: "messages.handle", NativeKey: "handle", Data: json.RawMessage(`{"address":"fixture@example.com","service":"iMessage"}`)},
	}}
	body, err := json.Marshal(dataset)
	if err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	req := httptest.NewRequest(http.MethodPost, "/dataset", strings.NewReader(string(body))).WithContext(ctx)
	response := httptest.NewRecorder()
	done := make(chan struct{})
	go func() {
		defer close(done)
		mux.ServeHTTP(response, req)
	}()
	var contacts captureRequest
	select {
	case contacts = <-f.changes:
	case <-ctx.Done():
		t.Fatal("contacts capture timeout")
	}
	f.mu.Lock()
	_, messagesVisible := f.records["handle"]
	f.mu.Unlock()
	close(contacts.Done)
	select {
	case messages := <-f.liveChanges:
		close(messages.Done)
	case <-ctx.Done():
		t.Fatal("messages capture timeout")
	}
	select {
	case <-done:
	case <-ctx.Done():
		t.Fatal("dataset response timeout")
	}
	if messagesVisible {
		t.Fatal("messages visible to scans before contacts capture completed")
	}
	if response.Code != http.StatusOK {
		t.Fatalf("dataset status: %d", response.Code)
	}
}
