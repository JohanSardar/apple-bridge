package scenarios

import (
	"context"
	"os"
	"path/filepath"
	"testing"
	"time"
)

func TestPrivateConnectReplicaUsesIsolatedHome(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	live := filepath.Join(home, "Library", "Application Support", "AppleBridgeLive")
	if err := os.MkdirAll(live, 0700); err != nil {
		t.Fatal(err)
	}
	sentinel := filepath.Join(live, "sentinel")
	if err := os.WriteFile(sentinel, []byte("untouched"), 0600); err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
	defer cancel()
	e := runPrivateConnect(ctx, Scenario{ID: "RMT-PRIVATE-CONNECT"}, "replica")
	if !e.Passed || e.Level != "replica" || e.RunID != "private-protocol" {
		t.Fatalf("isolated private protocol failed: %+v", e)
	}
	for _, a := range e.Assertions {
		if !a.Passed {
			t.Errorf("assertion failed: %s", a.Name)
		}
	}
	if data, err := os.ReadFile(sentinel); err != nil || string(data) != "untouched" {
		t.Fatalf("live sentinel changed: %q, %v", data, err)
	}
	entries, err := os.ReadDir(live)
	if err != nil || len(entries) != 1 {
		t.Fatalf("live home changed: %v, %v", entries, err)
	}
}
