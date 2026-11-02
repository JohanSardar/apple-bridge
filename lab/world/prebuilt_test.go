package world

import (
	"context"
	"os"
	"path/filepath"
	"testing"
)

func TestPrebuiltRequiresExecutable(t *testing.T) {
	path := filepath.Join(t.TempDir(), "bridge")
	if requireExecutable(path) == nil {
		t.Fatal("missing executable accepted")
	}
	if err := os.WriteFile(path, []byte("fixture"), 0600); err != nil {
		t.Fatal(err)
	}
	if requireExecutable(path) == nil {
		t.Fatal("non-executable accepted")
	}
	if err := os.Chmod(path, 0700); err != nil {
		t.Fatal(err)
	}
	if err := requireExecutable(path); err != nil {
		t.Fatal(err)
	}
}

func TestPrebuiltIsOptIn(t *testing.T) {
	ctx := context.Background()
	if prebuilt(ctx) {
		t.Fatal("default bypasses build")
	}
	if !prebuilt(WithPrebuilt(ctx)) {
		t.Fatal("prebuilt option lost")
	}
}
