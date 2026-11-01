package world

import (
	"context"
	"os"
	"path/filepath"
	"testing"
)

func TestIdleSessionKeepsProcessAndPrivateState(t *testing.T) {
	t.Setenv("APPLE_BRIDGE_SECRET_STORE", "file")
	if err := os.MkdirAll("/tmp/ablab", 0700); err != nil {
		t.Fatal(err)
	}
	directory, err := os.MkdirTemp("/tmp/ablab", "session-test-")
	if err != nil {
		t.Fatal(err)
	}
	w := &World{Dir: directory, HelperApp: filepath.Join(directory, "apps", "AppleBridgeHelper.app"), Gateway: Endpoint{Home: filepath.Join(directory, "gateway"), Token: "synthetic"}, ctx: context.Background()}
	defer w.Close()
	process, err := w.start("gateway", false, w.Gateway.Home, "/bin/sleep", "60")
	if err != nil {
		t.Fatal(err)
	}
	path := filepath.Join(directory, "session.json")
	if err = w.SaveSession(path, "revision"); err != nil {
		t.Fatal(err)
	}
	if err = w.SaveSession(path, "revision"); !os.IsExist(err) {
		t.Fatalf("overwrote session: %v", err)
	}
	if _, err = ResumeSession(context.Background(), path, "other"); err == nil {
		t.Fatal("accepted another revision")
	}
	if err = os.Chmod(path, 0644); err != nil {
		t.Fatal(err)
	}
	if _, err = ResumeSession(context.Background(), path, "revision"); err == nil {
		t.Fatal("accepted public session")
	}
	if err = os.Chmod(path, 0600); err != nil {
		t.Fatal(err)
	}
	resumed, err := ResumeSession(context.Background(), path, "revision")
	if err != nil {
		t.Fatal(err)
	}
	if resumed.Gateway.Token != "synthetic" || resumed.Dir != directory {
		t.Fatal("session changed")
	}
	resumed.Close()
	if _, err = os.Stat(path); !os.IsNotExist(err) {
		t.Fatal("session was not removed")
	}
	select {
	case <-process.done:
		process.stopped = true
	default:
		t.Fatal("gateway still running")
	}
}
