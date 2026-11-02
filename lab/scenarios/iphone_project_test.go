package scenarios

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestIPhoneProjectCommand(t *testing.T) {
	t.Setenv("APPLE_BRIDGE_IPHONE_ROOT", "")
	if err := iphoneProjectCommand(context.Background(), "pwd").Run(); err == nil || !strings.Contains(err.Error(), "APPLE_BRIDGE_IPHONE_ROOT") {
		t.Fatalf("missing checkout must explain configuration: %v", err)
	}
	root := filepath.Join(t.TempDir(), "phone checkout")
	if err := os.MkdirAll(filepath.Join(root, "ios"), 0700); err != nil {
		t.Fatal(err)
	}
	t.Setenv("APPLE_BRIDGE_IPHONE_ROOT", root)
	if err := iphoneProjectCommand(context.Background(), "pwd").Run(); err == nil {
		t.Fatal("unrelated directory accepted as iPhone checkout")
	}
	if err := os.WriteFile(filepath.Join(root, "ios", "project.yml"), []byte("name: AppleBridge\n"), 0600); err != nil {
		t.Fatal(err)
	}
	output, err := iphoneProjectCommand(context.Background(), "pwd", "-P").Output()
	actual, _ := filepath.EvalSymlinks(strings.TrimSpace(string(output)))
	want, _ := filepath.EvalSymlinks(root)
	if err != nil || actual != want {
		t.Fatalf("integration command ran outside the phone checkout: %q, %v", output, err)
	}
}
