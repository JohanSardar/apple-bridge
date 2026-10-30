package scenarios

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func runQualificationPreflight(ctx context.Context, s Scenario) Evidence {
	e := Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: "real", At: time.Now().UTC(), Status: "limited", Metrics: map[string]any{}}
	add := func(name string, passed bool) {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: passed})
	}
	dir, err := os.MkdirTemp("", "ab-qualification-")
	add("private_directory", err == nil)
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
	report := filepath.Join(os.Getenv("HOME"), "Library", "Application Support", "AppleBridgeDev", e.RunID+"-preflight.json")
	defer os.Remove(report)
	err = exec.CommandContext(ctx, "open", "-n", "-W", "-g", "-a", app, "--args", "qualification-preflight", s.ID, "--report", report).Run()
	add("signed_helper_probe", err == nil)
	data, readErr := os.ReadFile(report)
	var result struct {
		Granted bool `json:"grant_passed"`
		Code    int  `json:"automation_code"`
		Screen  bool `json:"screen_capture_preflight"`
	}
	add("helper_report", readErr == nil && json.Unmarshal(data, &result) == nil)
	if readErr != nil {
		return e
	}
	e.Metrics["grant_passed"] = result.Granted
	e.Metrics["writes_submitted"] = 0
	reason := "real Mail command fixture and owned-cleanup harness are not implemented"
	if s.ID == "WEB-REM-PARITY" || s.ID == "WEB-CAL-PARITY" {
		reason = "native parity command fixture and owned-cleanup harness are not implemented"
	}
	if s.ID == "CAP-SIM" {
		e.Metrics["screen_capture_preflight"] = result.Screen
		reason = "camera and screen-video providers remain unsupported; complete real CAP-SIM acceptance is unavailable"
		if !result.Granted {
			reason = "Screen Recording grant unavailable; no consent requested"
		}
	} else {
		e.Metrics["automation_code"] = result.Code
		if !result.Granted {
			reason = fmt.Sprintf("Automation preflight returned %d; no consent requested", result.Code)
		}
	}
	e.Conditions = map[string]string{"reason": reason}
	return e
}
