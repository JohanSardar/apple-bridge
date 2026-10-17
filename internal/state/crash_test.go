package state

import (
	"context"
	"encoding/json"
	"errors"
	"os"
	"os/exec"
	"testing"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

func TestApplyCrashBoundary(t *testing.T) {
	ctx := context.Background()
	home := os.Getenv("APPLE_BRIDGE_CRASH_HOME")
	record := contracts.Record{Collection: "reminders.reminder", ID: "rmd_fixture", Version: 1, Source: contracts.Reminders, ObservedAt: time.Now().UTC(), Data: json.RawMessage(`{"title":"[ab-test] before"}`)}
	batch := contracts.Batch{Epoch: "fixture-epoch", SubscriptionRevision: 1, After: 1, Through: 2}
	updated := record
	updated.Version = 2
	updated.Data = json.RawMessage(`{"title":"[ab-test] after"}`)
	batch.Changes = []contracts.Change{{Sequence: 2, Op: "upsert", Record: updated}}
	if home != "" {
		store, err := Open(home)
		if err != nil {
			t.Fatal(err)
		}
		store.applyBoundary = func() { os.Exit(91) }
		store.ApplyBatch(ctx, batch)
		t.Fatal("crash boundary not reached")
	}
	home = t.TempDir()
	store, err := Open(home)
	if err != nil {
		t.Fatal(err)
	}
	install, err := store.BeginInstall(ctx, contracts.FeedFrame{ID: "snapshot", Count: 1, Digest: Digest([]contracts.Record{record}), S: 1, ExpiresAt: time.Now().Add(time.Minute)}, batch.Epoch, 1)
	if err != nil {
		t.Fatal(err)
	}
	if err = install.Page(ctx, contracts.FeedFrame{ID: "snapshot", Records: []contracts.Record{record}}); err != nil {
		t.Fatal(err)
	}
	if _, err = install.InstallSnapshot(ctx); err != nil {
		t.Fatal(err)
	}
	store.Close()
	child := exec.Command(os.Args[0], "-test.run=^TestApplyCrashBoundary$")
	child.Env = append(os.Environ(), "APPLE_BRIDGE_CRASH_HOME="+home)
	err = child.Run()
	var exit *exec.ExitError
	if !errors.As(err, &exit) || exit.ExitCode() != 91 {
		t.Fatalf("unexpected crash result: %v", err)
	}
	store, err = Open(home)
	if err != nil {
		t.Fatal(err)
	}
	defer store.Close()
	result, err := store.Read(ctx, trust.AllVisibility(), contracts.ReadQuery{})
	if err != nil || result.Cursor.Sequence != 1 || len(result.Records) != 1 || result.Records[0].Version != 1 {
		t.Fatalf("partial transaction survived: %+v %v", result, err)
	}
	for range 2 {
		if _, err = store.ApplyBatch(ctx, batch); err != nil {
			t.Fatal(err)
		}
	}
	integrity, err := store.Integrity(ctx)
	if err != nil || !integrity.OK || !integrity.ExactlyOnce || integrity.Applied != 1 {
		t.Fatalf("replay integrity: %+v %v", integrity, err)
	}
}
