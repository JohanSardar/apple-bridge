package state

import (
	"context"
	"encoding/json"
	"testing"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

func TestPhotosCaptureDoesNotInterruptNotesEnvironment(t *testing.T) {
	store, err := Open(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	defer store.Close()
	ctx := context.Background()
	for _, capture := range []contracts.Capture{
		{Source: "photos", Generation: "ab-test-photos", Checkpoint: "1", Observations: []contracts.Observation{
			{Kind: "upsert", Collection: "photos.album", NativeKey: "ab-test-album", ObservedAt: time.Now().UTC(), Data: json.RawMessage(`{"name":"[ab-test] album","kind":"user","count":1}`)},
			{Kind: "upsert", Collection: "photos.asset", NativeKey: "ab-test-asset", ObservedAt: time.Now().UTC(), Data: json.RawMessage(`{"local_id":"ab-test-asset","created":null,"kind":"image","size":null,"duration":0,"favorite":false,"hidden":false,"location":null,"albums":["ab-test-album"],"cloud_state":"unknown"}`)},
		}},
		{Source: contracts.Notes, Generation: "ab-test-notes", Checkpoint: "1", Observations: []contracts.Observation{
			{Kind: "upsert", Collection: "notes.note", NativeKey: "ab-test-note", ObservedAt: time.Now().UTC(), Data: json.RawMessage(`{"title":"[ab-test] note","plain_text":"body"}`)},
		}},
	} {
		if _, err = store.CommitCapture(ctx, capture); err != nil {
			t.Fatalf("%s capture interrupts the native session: %v", capture.Source, err)
		}
	}
	for _, collection := range []string{"photos.album", "photos.asset", "notes.note"} {
		result, err := store.Read(ctx, trust.AllVisibility(), contracts.ReadQuery{Collection: collection})
		if err != nil || len(result.Records) != 1 {
			t.Fatalf("%s read-back count=%d: %v", collection, len(result.Records), err)
		}
	}
}
