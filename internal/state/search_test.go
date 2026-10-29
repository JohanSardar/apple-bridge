package state

import (
	"context"
	"encoding/json"
	"testing"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

func TestSearchUpgradeVersionFour(t *testing.T) {
	ctx := context.Background()
	home := t.TempDir()
	store, err := Open(home)
	if err != nil {
		t.Fatal(err)
	}
	_, err = store.CommitCapture(ctx, contracts.Capture{Source: contracts.Mail, Generation: "fixture", Checkpoint: "1", Observations: []contracts.Observation{{Kind: "upsert", Collection: "mail.message", NativeKey: "fixture", ObservedAt: time.Now().UTC(), Data: json.RawMessage(`{"subject":"subjectneedle","from":"senderneedle","to":"recipientneedle","snippet":"snippetneedle"}`)}}})
	if err != nil {
		t.Fatal(err)
	}
	before, err := store.Read(ctx, trust.AllVisibility(), contracts.ReadQuery{})
	if err != nil {
		t.Fatal(err)
	}
	err = func() error {
		store.write.Lock()
		defer store.write.Unlock()
		_, err := store.db.Exec(dropSearchTriggers + `DELETE FROM records_fts; INSERT INTO records_fts(rowid,collection,id,source,text) SELECT m.fts_rowid,r.collection,r.id,r.source,'' FROM records r JOIN records_fts_map m ON m.collection=r.collection AND m.id=r.id WHERE r.deleted=0; UPDATE meta SET value='4' WHERE key='fts_version'`)
		return err
	}()
	if err != nil {
		t.Fatal(err)
	}
	store.Close()
	store, err = Open(home)
	if err != nil {
		t.Fatal(err)
	}
	defer store.Close()
	select {
	case <-store.searchDone:
	case <-time.After(10 * time.Second):
		t.Fatal("search rebuild timeout")
	}
	var version string
	if err = store.db.QueryRow("SELECT value FROM meta WHERE key='fts_version'").Scan(&version); err != nil || version != "5" {
		t.Fatalf("search version %q: %v", version, err)
	}
	for _, term := range []string{"subjectneedle", "senderneedle", "recipientneedle", "snippetneedle"} {
		var count int
		if err = store.db.QueryRow("SELECT count(*) FROM records_fts WHERE records_fts MATCH ?", term).Scan(&count); err != nil || count != 1 {
			t.Fatalf("mail projection count=%d: %v", count, err)
		}
	}
	after, err := store.Read(ctx, trust.AllVisibility(), contracts.ReadQuery{})
	if err != nil || Digest(before.Records) != Digest(after.Records) {
		t.Fatalf("records changed: %v", err)
	}
}
