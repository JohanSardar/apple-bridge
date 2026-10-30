package scenarios

import (
	"context"
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/shortcut"
	"os"
	"os/exec"
	"path/filepath"
)

func runFocusReal(ctx context.Context, s Scenario, e Evidence) Evidence {
	add := func(name string, ok bool) {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok})
		e.Passed = e.Passed && ok
	}
	dir, err := os.MkdirTemp("", "ab-focus-real-")
	add("helper_directory", err == nil)
	if err != nil {
		return e
	}
	defer os.RemoveAll(dir)
	app := filepath.Join(dir, "AppleBridgeHelper.app")
	output, err := exec.CommandContext(ctx, "packaging/dev/build-helper.sh", app).CombinedOutput()
	add("signed_helper_build", err == nil)
	if err != nil {
		os.Stderr.Write(output)
		return e
	}
	report := filepath.Join(os.Getenv("HOME"), "Library", "Application Support", "AppleBridgeDev", "focus-"+e.RunID+".json")
	defer os.Remove(report)
	output, err = exec.CommandContext(ctx, "open", "-n", "-W", "-a", app, "--args", "focus-real-checks", "FOC-STATUS-REAL", "--report", report).CombinedOutput()
	add("signed_helper_probe", err == nil)
	if err != nil {
		os.Stderr.Write(output)
		return e
	}
	raw, err := os.ReadFile(report)
	var result map[string]bool
	if err == nil {
		err = json.Unmarshal(raw, &result)
	}
	add("helper_report", err == nil)
	if err != nil {
		return e
	}
	granted, known := result["grant_passed"]
	add("grant_probe", known)
	if !known {
		return e
	}
	if !granted {
		e.Status = "limited"
		e.Passed = false
		e.Conditions = map[string]string{"reason": "needs Focus status grant for AppleBridgeHelper"}
		return e
	}
	if s.ID == "FOC-STATUS-REAL" {
		add("status_readback", result["status_passed"])
		return e
	}
	entries, err := (shortcut.CLI{}).Inventory(ctx, false, "")
	if err != nil {
		e.Status = "limited"
		e.Passed = false
		e.Conditions = map[string]string{"reason": "needs Shortcuts inventory access"}
		return e
	}
	count := 0
	for _, entry := range entries {
		if entry.Name == "AB Focus" {
			count++
		}
	}
	e.Metrics["pack_count"] = count
	if count != 1 {
		e.Status = "limited"
		e.Passed = false
		e.Conditions = map[string]string{"reason": "needs exactly one imported AB Focus pack"}
		return e
	}
	if err = os.Remove(report); err != nil {
		add("clear_status_report", false)
		return e
	}
	output, err = exec.CommandContext(ctx, "open", "-n", "-W", "-a", app, "--args", "focus-real-checks", s.ID, "--report", report).CombinedOutput()
	add("signed_set_restore_probe", err == nil)
	if err != nil {
		os.Stderr.Write(output)
		return e
	}
	raw, err = os.ReadFile(report)
	result = nil
	if err == nil {
		err = json.Unmarshal(raw, &result)
	}
	add("set_restore_report", err == nil)
	var failure struct {
		Failure string `json:"failure"`
	}
	if json.Unmarshal(raw, &failure) == nil && failure.Failure != "" {
		e.Assertions = append(e.Assertions, Assertion{Name: "native_set_restore_completed", Passed: false, Detail: failure.Failure})
		e.Passed = false
		return e
	}
	if err != nil {
		return e
	}
	if !result["inactive_passed"] {
		e.Status = "limited"
		e.Passed = false
		e.Conditions = map[string]string{"reason": "needs initially inactive Focus to preserve the prior mode and expiry"}
		return e
	}
	add("set_readback", result["set_passed"])
	add("restore_readback", result["restore_passed"])
	return e
}
