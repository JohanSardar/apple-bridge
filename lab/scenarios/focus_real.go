package scenarios

import (
	"context"
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/shortcut"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
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
	var output []byte
	if selected := os.Getenv("AB_REAL_FOCUS_HELPER"); s.ID == "FOC-PHONE-SYNC" && selected != "" {
		app = selected
		output, err = exec.CommandContext(ctx, "codesign", "--verify", "--strict", app).CombinedOutput()
	} else {
		build := exec.CommandContext(ctx, "packaging/dev/build-helper.sh", app)
		if s.ID == "FOC-PHONE-SYNC" {
			if sdk, sdkErr := exec.CommandContext(ctx, "packaging/dev/sdkroot.sh").Output(); sdkErr == nil && len(sdk) > 0 {
				build.Env = append(os.Environ(), "SDKROOT="+strings.TrimSpace(string(sdk)))
			}
		}
		output, err = build.CombinedOutput()
	}
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
	var result map[string]any
	if err == nil {
		err = json.Unmarshal(raw, &result)
	}
	add("helper_report", err == nil)
	if err != nil {
		return e
	}
	granted, known := result["grant_passed"].(bool)
	add("grant_probe", known)
	if !known {
		return e
	}
	if !granted {
		add("focus_status_authorized", false)
		e.Status = "limited"
		e.Passed = false
		e.Conditions = map[string]string{"reason": "needs Focus status grant for AppleBridgeHelper"}
		return e
	}
	if s.ID == "FOC-STATUS-REAL" {
		add("status_readback", result["status_passed"] == true)
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
	if err != nil {
		return e
	}
	if result["inactive_passed"] != true {
		e.Status = "limited"
		e.Passed = false
		e.Conditions = map[string]string{"reason": "needs initially inactive Focus to preserve the prior mode and expiry"}
		return e
	}
	add("set_readback", result["set_passed"] == true)
	add("restore_readback", result["restore_passed"] == true)
	for _, name := range []string{"pack_binding_readback", "set_shortcut_completed", "set_shortcut_not_timed_out", "restore_shortcut_completed", "restore_shortcut_not_timed_out", "set_status_after_shortcut", "restore_status_after_shortcut"} {
		if passed, exists := result[name].(bool); exists {
			add(name, passed)
		}
	}
	for _, stage := range []string{"pack_repair_error", "prepare_error", "set_error", "restore_error"} {
		if detail, ok := result[stage].(string); ok {
			if e.Conditions == nil {
				e.Conditions = map[string]string{}
			}
			e.Conditions[stage] = detail
		}
	}
	for _, prefix := range []string{"set_", "restore_"} {
		if detail, ok := result[prefix+"cli_error_category"].(string); ok {
			if e.Conditions == nil {
				e.Conditions = map[string]string{}
			}
			e.Conditions[prefix+"cli_error_category"] = detail
			e.Metrics[prefix+"cli_error_bytes"] = result[prefix+"cli_error_bytes"]
		}
	}
	return e
}
