package scenarios

import (
	"context"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"sort"
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
		flag, marker := "--section-checks", "section checks passed"
		if e.Scenario == "WEB-REM-PARITY" {
			flag, marker = "--parity-checks", "parity checks passed"
		}
		output, err = exec.CommandContext(ctx, filepath.Join(strings.TrimSpace(string(directory)), "RemindersStoreChecks"), flag).CombinedOutput()
		add("store_effects_and_invalidation", err == nil && strings.Contains(string(output), marker))
	} else {
		report = filepath.Join(os.Getenv("HOME"), "Library", "Application Support", "AppleBridgeDev", "sections-"+e.RunID+".json")
		defer os.Remove(report)
		err = exec.CommandContext(ctx, "open", "-n", "-W", "-g", "-a", filepath.Join(dir, "AppleBridgeHelper.app"), "--args", remindersProofCommand(e.Scenario), "--report", report).Run()
		data, problem := os.ReadFile(report)
		var proof struct {
			Passed  bool              `json:"passed"`
			Checks  map[string]bool   `json:"checks"`
			Reason  string            `json:"reason"`
			Reasons map[string]string `json:"reasons"`
		}
		valid := err == nil && problem == nil && json.Unmarshal(data, &proof) == nil
		add("native_sections_report", valid)
		names := make([]string, 0, len(proof.Checks))
		for name := range proof.Checks {
			names = append(names, name)
		}
		sort.Strings(names)
		for _, name := range names {
			add(name, proof.Checks[name])
		}
		if e.Scenario == "WEB-REM-PARITY" {
			for _, name := range []string{"flag_readback", "flag_clear_readback", "list_icon_readback", "indent_readback", "outdent_readback", "tags_readback", "reorder_readback", "image_readback", "smart_list_readback", "owned_list_cleanup"} {
				if _, exists := proof.Checks[name]; !exists {
					add(name, false)
				}
			}
		}
		add("native_sections_proof", valid && proof.Passed)
		e.Conditions = proof.Reasons
		if proof.Reason != "" {
			if e.Conditions == nil {
				e.Conditions = map[string]string{}
			}
			e.Conditions["reason"] = proof.Reason
		}
	}
	e.Passed = len(e.Assertions) > 0
	for _, check := range e.Assertions {
		e.Passed = e.Passed && check.Passed
	}
	return e
}

func remindersProofCommand(scenario string) string {
	if scenario == "WEB-REM-PARITY" {
		return "reminders-parity-proof"
	}
	return "reminders-sections-proof"
}
