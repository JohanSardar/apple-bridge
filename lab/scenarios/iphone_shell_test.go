package scenarios

import (
	"context"
	"os/exec"
	"runtime"
	"testing"
	"time"
)

func TestIPhoneShellSimUsesAppViewModels(t *testing.T) {
	if runtime.GOOS != "darwin" {
		t.Skip("requires the local Swift toolchain")
	}
	if _, err := exec.LookPath("xcrun"); err != nil {
		t.Skip("requires xcrun")
	}
	t.Chdir("../..")
	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Minute)
	defer cancel()
	e := runScenario(ctx, Scenario{ID: "APP-SHELL-SIM", Feature: "iphone.shell"}, "sim")
	if !e.Passed {
		t.Fatalf("shell view models failed: %+v", e.Assertions)
	}
	found := map[string]bool{}
	for _, assertion := range e.Assertions {
		found[assertion.Name] = assertion.Passed
	}
	for _, name := range []string{"tab_structure", "offline_status", "reconnect_status", "approval_list_rendering", "approval_expiry_refresh"} {
		if !found[name] {
			t.Errorf("shell route omitted %s", name)
		}
	}
	if found["convergence_truth_gateway_replicas"] {
		t.Fatal("shell route fell back to generic replication")
	}
}

func TestIPhoneShellSimFailsWithoutSwiftProof(t *testing.T) {
	t.Chdir(t.TempDir())
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	e := runScenario(ctx, Scenario{ID: "APP-SHELL-SIM", Feature: "iphone.shell"}, "sim")
	if e.Passed {
		t.Fatal("shell scenario accepted a missing native proof")
	}
}
