package world

import (
	"context"
	"os"
	"path/filepath"
	"testing"
	"time"
)

func TestCommandOutputDoesNotWaitForInheritedBuildPipes(t *testing.T) {
	marker := filepath.Join(t.TempDir(), "finished")
	started := time.Now()
	output, err := commandOutput(context.Background(), "sh", "-c", `set -e; (sleep 3; printf child >&2; printf alive > "$1") & printf ready`, "build", marker)
	if err != nil || output != "ready" || time.Since(started) >= 2*time.Second {
		t.Fatalf("build descendants kept output open: output=%q error=%v", output, err)
	}
	deadline := time.Now().Add(5 * time.Second)
	for {
		if data, err := os.ReadFile(marker); err == nil && string(data) == "alive" {
			return
		}
		if time.Now().After(deadline) {
			t.Fatal("detached build could not finish after its caller closed output")
		}
		time.Sleep(20 * time.Millisecond)
	}
}
