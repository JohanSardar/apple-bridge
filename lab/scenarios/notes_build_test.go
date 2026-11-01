package scenarios

import (
	"context"
	"os"
	"path/filepath"
	"testing"
	"time"

	"github.com/JohanSardar/apple-bridge/lab/world"
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

func TestNotesTruthBuildDoesNotWaitForTheSharedHelperLane(t *testing.T) {
	root := t.TempDir()
	t.Chdir(root)
	for _, path := range []string{"packaging/dev", "lab/truth/native", "shared-helper.lock"} {
		if err := os.MkdirAll(path, 0700); err != nil {
			t.Fatal(err)
		}
	}
	helperLock := filepath.Join(root, "shared-helper.lock")
	t.Setenv("APPLE_BRIDGE_NATIVE_LOCK", helperLock)
	script := `#!/bin/sh
until mkdir "$APPLE_BRIDGE_NATIVE_LOCK" 2>/dev/null; do sleep 1; done
trap 'rmdir "$APPLE_BRIDGE_NATIVE_LOCK"' EXIT
mkdir -p "$1"
printf '%s' "$APPLE_BRIDGE_NATIVE_LOCK" > "$1/build-lock"
`
	if err := os.WriteFile("packaging/dev/build-truth.sh", []byte(script), 0700); err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()
	app := filepath.Join(root, "AppleTruth.app")
	setup := notesNativeSetup(ctx)
	if !setup.IndependentTruth {
		t.Fatal("shared Truth build still enabled")
	}
	if err := setup.BeforeStart(&world.World{TruthApp: app}); err != nil {
		t.Fatal(err)
	}
	lock, err := os.ReadFile(filepath.Join(app, "build-lock"))
	if err != nil {
		t.Fatal(err)
	}
	if string(lock) == helperLock || os.Getenv("APPLE_BRIDGE_NATIVE_LOCK") != helperLock {
		t.Fatal("Truth build reused or changed the Helper lock")
	}
	if _, err := os.Stat(helperLock); err != nil {
		t.Fatal("Truth build removed the Helper lock")
	}
	if _, err := os.Stat(string(lock)); !os.IsNotExist(err) {
		t.Fatal("Truth build left its private lock behind")
	}
}
