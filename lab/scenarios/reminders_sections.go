package scenarios

import (
	"context"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
)

func runRemindersSections(ctx context.Context, level string, e Evidence) Evidence {
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	dir, err := os.MkdirTemp("", "ab-sections-")
	if err != nil {
		add("proof_directory", false)
		return e
	}
	defer os.RemoveAll(dir)
	var command *exec.Cmd
	var report string
	if level == "sim" {
		command = exec.CommandContext(ctx, "packaging/dev/build-native.sh")
		command.Env = append(os.Environ(), "APPLE_BRIDGE_NATIVE_TARGET=RemindersStoreChecks")
	} else {
		command = exec.CommandContext(ctx, "packaging/dev/build-helper.sh", filepath.Join(dir, "AppleBridgeHelper.app"))
	}
	output, err := command.CombinedOutput()
	add("native_sections_build", err == nil)
	if err != nil {
		os.Stderr.Write(output)
		return e
	}
	if level == "sim" {
		directory, problem := os.ReadFile("native/.build/native-output-1")
		if problem != nil {
			add("native_sections_binary", false)
			return e
		}
		output, err = exec.CommandContext(ctx, filepath.Join(strings.TrimSpace(string(directory)), "RemindersStoreChecks"), "--section-checks").CombinedOutput()
		add("store_section_order_membership_and_invalidation", err == nil && strings.Contains(string(output), "section checks passed"))
	} else {
		report = filepath.Join(os.Getenv("HOME"), "Library", "Application Support", "AppleBridgeDev", "sections-"+e.RunID+".json")
		defer os.Remove(report)
		err = exec.CommandContext(ctx, "open", "-n", "-W", "-g", "-a", filepath.Join(dir, "AppleBridgeHelper.app"), "--args", "reminders-sections-proof", "--report", report).Run()
		data, problem := os.ReadFile(report)
		var proof struct {
			Passed bool            `json:"passed"`
			Checks map[string]bool `json:"checks"`
			Reason string          `json:"reason"`
		}
		valid := err == nil && problem == nil && json.Unmarshal(data, &proof) == nil
		add("native_sections_report", valid)
		for name, passed := range proof.Checks {
			add(name, passed)
		}
		add("native_sections_proof", valid && proof.Passed)
		if proof.Reason != "" {
			e.Conditions = map[string]string{"reason": proof.Reason}
		}
	}
	e.Passed = len(e.Assertions) > 0
	for _, check := range e.Assertions {
		e.Passed = e.Passed && check.Passed
	}
	return e
}
