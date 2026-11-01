package scenarios

import (
	"os"
	"path/filepath"
	"testing"
)

func TestNotesNativeBuildLockMatchesSharedLane(t *testing.T) {
	t.Setenv("APPLE_BRIDGE_NATIVE_LOCK", "")
	root := t.TempDir()
	lane := filepath.Join(root, "ab-swift-build-2")
	if err := os.Mkdir(lane, 0700); err != nil {
		t.Fatal(err)
	}
	build := filepath.Join(root, ".build")
	if err := os.Symlink(lane, build); err != nil {
		t.Fatal(err)
	}
	restore := notesNativeBuildLock(build)
	if got := os.Getenv("APPLE_BRIDGE_NATIVE_LOCK"); got != "/tmp/ab-native-lane-2.lock" {
		t.Fatalf("shared build lock = %q", got)
	}
	restore()
	if got, present := os.LookupEnv("APPLE_BRIDGE_NATIVE_LOCK"); !present || got != "" {
		t.Fatalf("original build lock was not restored: %q, %t", got, present)
	}
	t.Setenv("APPLE_BRIDGE_NATIVE_LOCK", filepath.Join(root, "explicit.lock"))
	restore = notesNativeBuildLock(build)
	restore()
	if got := os.Getenv("APPLE_BRIDGE_NATIVE_LOCK"); got != filepath.Join(root, "explicit.lock") {
		t.Fatal("explicit build lock changed")
	}
}
