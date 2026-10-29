package scenarios

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"syscall"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func runNotesParity(ctx context.Context, s Scenario, level string) (e Evidence) {
	e = Evidence{RunID: contracts.NewID("notes-parity"), Scenario: s.ID, Level: level, At: time.Now().UTC(), Hosts: []string{"air"}, Metrics: map[string]any{}}
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	defer func() {
		e.Passed = len(e.Assertions) > 0
		for _, assertion := range e.Assertions {
			e.Passed = e.Passed && assertion.Passed
		}
	}()
	if level != "real" {
		add("real_account_required", false)
		return
	}
	before, problem := cleanupSnapshot(ctx, "notes")
	if problem != nil {
		e.Status = "limited"
		e.Assertions = append(e.Assertions, Assertion{Name: "cleanup_baseline", Status: "limited", Detail: "needs Notes Full Disk Access"})
		return
	}
	owned, problem := newRunCleanup(e.RunID, before)
	if problem != nil {
		add("cleanup_baseline", false)
		return
	}
	defer func() { finishCleanupAudit(context.Background(), owned, "notes", &e) }()
	root, err := os.MkdirTemp("", "ab-test-notes-parity-")
	if err != nil {
		add("fixture_directory", false)
		return
	}
	defer os.RemoveAll(root)
	app := filepath.Join(root, "AppleBridgeHelper.app")
	cmd := exec.CommandContext(ctx, "packaging/dev/build-helper.sh", app)
	cmd.Env = append(os.Environ(), "APPLE_BRIDGE_NATIVE_LAB=1")
	cmd.SysProcAttr = &syscall.SysProcAttr{Setpgid: true}
	cmd.Cancel = func() error { return syscall.Kill(-cmd.Process.Pid, syscall.SIGKILL) }
	output, err := cmd.CombinedOutput()
	if err != nil {
		fmt.Fprintln(os.Stderr, string(output))
		add("signed_helper_build", false)
		return
	}
	home, _ := os.UserHomeDir()
	report := filepath.Join(home, "Library/Application Support/AppleBridgeDev", e.RunID+".json")
	defer os.Remove(report)
	cmd = exec.CommandContext(ctx, "open", "-n", "-W", "-g", "-a", app, "--args", "notes-parity-probe", "--report", report)
	output, err = cmd.CombinedOutput()
	if err != nil {
		fmt.Fprintln(os.Stderr, string(output))
		add("signed_helper_probe", false)
		return
	}
	bytes, err := os.ReadFile(report)
	var result struct {
		State         string                     `json:"state"`
		OS            string                     `json:"os"`
		Cleanup       string                     `json:"cleanup"`
		MutationCalls int                        `json:"mutation_calls"`
		Proofs        map[string]json.RawMessage `json:"proofs"`
	}
	if err != nil || json.Unmarshal(bytes, &result) != nil {
		add("private_report", false)
		return
	}
	e.Metrics["os"] = result.OS
	e.Metrics["mutation_calls"] = result.MutationCalls
	add("tagged_notes_cleaned_up", result.Cleanup == "complete")
	for _, name := range []string{"simple_rich", "checklist"} {
		var proof struct {
			State  string `json:"state"`
			Reason string `json:"reason"`
		}
		err := json.Unmarshal(result.Proofs[name], &proof)
		e.Assertions = append(e.Assertions, Assertion{Name: name + "_capability", Passed: err == nil && (proof.State == "candidate" || proof.State == "limited"), Status: "limited", Detail: proof.Reason})
		e.Metrics[name] = json.RawMessage(result.Proofs[name])
	}
	return
}
