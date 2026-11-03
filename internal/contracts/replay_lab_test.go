//go:build lab

package contracts

import (
	"os"
	"path/filepath"
	"testing"
	"time"
)

func TestRecordedDecisions(t *testing.T) {
	path := filepath.Join(t.TempDir(), "decisions.json")
	t.Setenv("APPLE_BRIDGE_LAB_REPLAY", path)
	write := func(raw string) {
		t.Helper()
		if err := os.WriteFile(path, []byte(raw), 0600); err != nil {
			t.Fatal(err)
		}
	}
	write(`{"now":"2026-11-03T12:00:00.123456789Z","aliases":{"reminders\u0000generation\u0000reminders.list\u0000native":"rl_recorded"}}`)
	if got := DecisionNow().Format(time.RFC3339Nano); got != "2026-11-03T12:00:00.123456789Z" {
		t.Fatal(got)
	}
	if id, ok := ReplayRecordID(Reminders, "generation", "reminders.list", "native"); !ok || id != "rl_recorded" {
		t.Fatal(id, ok)
	}
	write(`{"now":"2026-11-03T12:05:00Z","aliases":{}}`)
	if got := DecisionNow().Format(time.RFC3339Nano); got != "2026-11-03T12:05:00Z" {
		t.Fatal(got)
	}
	mustPanic := func(f func()) {
		t.Helper()
		defer func() {
			if recover() == nil {
				t.Error("replay silently used an unrecorded decision")
			}
		}()
		f()
	}
	mustPanic(func() { ReplayRecordID(Reminders, "generation", "reminders.list", "missing") })
	write(`{"now":"broken"}`)
	mustPanic(func() { DecisionNow() })
	write(`{`)
	mustPanic(func() { DecisionNow() })
	os.Remove(path)
	mustPanic(func() { DecisionNow() })
}
