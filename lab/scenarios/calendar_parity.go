package scenarios

import (
	"context"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"sort"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func runCalendarParity(ctx context.Context, scenario Scenario, level string) (e Evidence) {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e = Evidence{RunID: contracts.NewID("calendar-parity"), Scenario: scenario.ID, Level: level, GitSHA: strings.TrimSpace(string(sha)), At: time.Now().UTC(), Assertions: []Assertion{}, Metrics: map[string]any{}}
	add := func(name string, passed bool) {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: passed})
	}
	defer func() {
		e.Passed = len(e.Assertions) > 0
		for _, check := range e.Assertions {
			e.Passed = e.Passed && check.Passed
		}
	}()
	root, err := os.MkdirTemp("", "ab-test-calendar-parity-")
	if err != nil {
		add("private_directory", false)
		return
	}
	defer os.RemoveAll(root)
	if level == "sim" {
		build := exec.CommandContext(ctx, "packaging/dev/build-native.sh")
		build.Env = append(os.Environ(), "APPLE_BRIDGE_NATIVE_TARGET=CalendarChecks")
		output, err := build.CombinedOutput()
		add("calendar_checks_build", err == nil)
		if err != nil {
			os.Stderr.Write(output)
			return
		}
		directory, err := os.ReadFile("native/.build/native-output-1")
		if err != nil {
			add("calendar_checks_binary", false)
			return
		}
		output, err = exec.CommandContext(ctx, filepath.Join(strings.TrimSpace(string(directory)), "CalendarChecks"), "--parity").CombinedOutput()
		add("calendar_parity_store_effects_and_cleanup", err == nil && strings.Contains(string(output), "calendar parity checks passed"))
		if err != nil {
			os.Stderr.Write(output)
		}
		return
	}
	e.Hosts = []string{"air"}
	app := filepath.Join(root, "AppleBridgeHelper.app")
	output, err := exec.CommandContext(ctx, "packaging/dev/build-helper.sh", app).CombinedOutput()
	add("signed_helper_build", err == nil)
	if err != nil {
		os.Stderr.Write(output)
		return
	}
	home, _ := os.UserHomeDir()
	report := filepath.Join(home, "Library/Application Support/AppleBridgeDev", e.RunID+".json")
	defer os.Remove(report)
	output, err = exec.CommandContext(ctx, "open", "-n", "-W", "-g", "-a", app, "--args", "calendar-parity-proof", "--report", report).CombinedOutput()
	if err != nil {
		os.Stderr.Write(output)
	}
	data, readErr := os.ReadFile(report)
	var proof struct {
		Passed   bool              `json:"passed"`
		Checks   map[string]bool   `json:"checks"`
		Reasons  map[string]string `json:"reasons"`
		Controls map[string]bool   `json:"controls"`
	}
	valid := err == nil && readErr == nil && json.Unmarshal(data, &proof) == nil
	add("native_parity_report", valid)
	names := make([]string, 0, len(proof.Checks))
	for name := range proof.Checks {
		names = append(names, name)
	}
	sort.Strings(names)
	for _, name := range names {
		add(name, proof.Checks[name])
	}
	for _, name := range []string{"invitation_response_readback", "travel_time_readback", "attachment_add_readback", "attachment_remove_readback", "owned_calendar_cleanup"} {
		if _, exists := proof.Checks[name]; !exists {
			add(name, false)
		}
	}
	add("native_calendar_parity", valid && proof.Passed)
	e.Conditions = proof.Reasons
	e.Metrics["controls"] = proof.Controls
	return
}
