package scenarios

import (
	"context"
	"os"
	"path/filepath"
	"testing"

	"github.com/JohanSardar/apple-bridge/lab/world"
)

func TestIdleStatusFailureHasOwnedDiagnostics(t *testing.T) {
	directory := t.TempDir()
	t.Setenv("HOME", directory)
	t.Setenv("PATH", directory)
	if err := os.WriteFile(filepath.Join(directory, "open"), []byte("#!/bin/sh\nprintf 'synthetic launch failure' >&2\nexit 1\n"), 0700); err != nil {
		t.Fatal(err)
	}
	e := Evidence{RunID: "idle-status-unit"}
	if awaitIdleCapture(context.Background(), &world.World{HelperApp: "synthetic.app"}, &e) {
		t.Fatal("unavailable status established idle")
	}
	if e.Metrics["idle_status_command_succeeded"] != false || e.Metrics["idle_status_report_present"] != false || e.Metrics["idle_status_context_cancelled"] != false {
		t.Fatalf("missing failure classification: %v", e.Metrics)
	}
	if len(e.Assertions) != 1 || e.Assertions[0].Name != "non_prompting_idle_permission_status" || e.Assertions[0].Passed {
		t.Fatalf("missing readiness failure: %v", e.Assertions)
	}
	path := filepath.Join("lab/evidence/private/idle", e.RunID+"-status.log")
	t.Cleanup(func() { os.Remove(path) })
	raw, err := os.ReadFile(path)
	if err != nil || len(raw) == 0 {
		t.Fatalf("missing private diagnostic: %v", err)
	}
}
