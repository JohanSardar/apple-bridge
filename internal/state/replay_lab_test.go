//go:build lab

package state

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"testing"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

func TestReplayEmbeddedIdentity(t *testing.T) {
	manifest := filepath.Join(t.TempDir(), "decisions.json")
	raw := `{"now":"2026-11-03T12:00:00Z","aliases":{"reminders\u0000fixture\u0000reminders.list\u0000list":"rl_recorded","reminders\u0000fixture\u0000reminders.reminder\u0000item":"ri_recorded"}}`
	if err := os.WriteFile(manifest, []byte(raw), 0600); err != nil {
		t.Fatal(err)
	}
	t.Setenv("APPLE_BRIDGE_LAB_REPLAY", manifest)
	store, err := Open(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	defer store.Close()
	_, err = store.CommitCapture(context.Background(), contracts.Capture{Source: contracts.Reminders, Generation: "fixture", Checkpoint: "1", Observations: []contracts.Observation{
		{Kind: "upsert", ObservedAt: contracts.DecisionNow(), Collection: "reminders.list", NativeKey: "list", Data: json.RawMessage(`{"title":"fixture"}`)},
		{Kind: "upsert", ObservedAt: contracts.DecisionNow(), Collection: "reminders.reminder", NativeKey: "item", Data: json.RawMessage(`{"list_id":{"$ref":{"collection":"reminders.list","native_key":"list"}},"title":"fixture"}`)},
	}})
	if err != nil {
		t.Fatal(err)
	}
	result, err := store.Read(context.Background(), trust.AllVisibility(), contracts.ReadQuery{Collection: "reminders.reminder"})
	if err != nil || len(result.Records) != 1 {
		t.Fatal(result, err)
	}
	if result.Records[0].ID != "ri_recorded" {
		t.Fatal(result.Records[0].ID)
	}
	var data map[string]any
	if err := json.Unmarshal(result.Records[0].Data, &data); err != nil {
		t.Fatal(err)
	}
	if data["list_id"] != "rl_recorded" {
		t.Fatal(data)
	}
	var committed int64
	if err := store.db.QueryRow("SELECT min(committed_at) FROM journal").Scan(&committed); err != nil || committed != 1793707200 {
		t.Fatal(committed, err)
	}
}
