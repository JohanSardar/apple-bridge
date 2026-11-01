package scenarios

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"syscall"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func runNotesParity(ctx context.Context, s Scenario, level string) (e Evidence) {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e = Evidence{GitSHA: strings.TrimSpace(string(sha)), RunID: contracts.NewID("notes-parity"), Scenario: s.ID, Level: level, At: time.Now().UTC(), Hosts: []string{"air"}, Metrics: map[string]any{}}
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	defer func() {
		e.Passed = len(e.Assertions) > 0
		for _, assertion := range e.Assertions {
			e.Passed = e.Passed && assertion.Passed
		}
	}()
	for _, id := range []string{"NOTE-CRUD", "NOTE-RICH-REJECT"} {
		result := runNotes(ctx, Scenario{ID: id}, level)
		if ids, ok := result.Metrics["notes_helper_binary_uuids"].([]string); ok {
			for _, id := range ids {
				notesAddHelperBinary(&e, id)
			}
		}
		for _, assertion := range result.Assertions {
			assertion.Name = id + "/" + assertion.Name
			e.Assertions = append(e.Assertions, assertion)
		}
		if !result.Passed {
			return
		}
	}
	if level == "sim" {
		return
	}
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
	defer func() {
		finishCleanupAudit(context.Background(), owned, "notes", &e)
		proof, ok := e.Metrics["cleanup_counts"].(map[string]any)
		if ok && proof["passed"] == true {
			for i := range e.Assertions {
				if e.Assertions[i].Name == "tagged_notes_cleaned_up" && !e.Assertions[i].Passed {
					e.Assertions[i].Passed = true
					e.Assertions[i].Detail = "independent_note_and_folder_baseline_restored_after_probe_exit"
				}
			}
		}
	}()
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
	e.Metrics["native_cleanup_reported_complete"] = result.Cleanup == "complete"
	add("tagged_notes_cleaned_up", result.Cleanup == "complete")
	for _, name := range []string{"simple_rich", "checklist"} {
		var proof struct {
			State  string `json:"state"`
			Reason string `json:"reason"`
		}
		err := json.Unmarshal(result.Proofs[name], &proof)
		passed := err == nil && (proof.State == "candidate" || proof.State == "limited")
		status := "limited"
		if name == "simple_rich" {
			passed = err == nil && proof.State == "candidate"
			status = ""
		}
		e.Assertions = append(e.Assertions, Assertion{Name: name + "_capability", Passed: passed, Status: status, Detail: proof.Reason})
		e.Metrics[name] = json.RawMessage(result.Proofs[name])
	}
	return
}
