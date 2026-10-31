package scenarios

import (
	"context"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func runSetup(ctx context.Context, s Scenario, level string) (e Evidence) {
	e = Evidence{Scenario: s.ID, Level: level, At: time.Now().UTC(), RunID: contracts.NewID("setup")}
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	defer func() {
		e.Passed = len(e.Assertions) > 0
		for _, assertion := range e.Assertions {
			e.Passed = e.Passed && assertion.Passed
		}
	}()
	if level == "sim" {
		output, err := exec.CommandContext(ctx, "packaging/setup/check.sh").CombinedOutput()
		add("setup_sim_checks", err == nil)
		for _, name := range []string{"CATALOG", "FRESH", "COMPLETE", "ALL-SKIPPED", "PERSIST", "REOPEN", "UNKNOWN", "LIMITED"} {
			add(strings.ToLower(name), strings.Contains(string(output), "Q1-"+name+" passed=true"))
		}
		if err != nil {
			os.Stderr.Write(output)
		}
		return
	}
	if runtime.GOOS != "darwin" {
		add("macos_required", false)
		return
	}
	root, err := os.MkdirTemp("", "ab-setup-real-")
	if err != nil {
		add("isolated_setup_home", false)
		return
	}
	defer os.RemoveAll(root)
	home := filepath.Join(root, "setup-data")
	evidence, err := filepath.Abs(filepath.Join("lab/evidence/private/setup", e.RunID))
	if err != nil {
		add("snapshot_directory", false)
		return
	}
	app := filepath.Join(root, "AppleBridgeHelper.app")
	output, err := exec.CommandContext(ctx, "packaging/dev/build-helper.sh", app).CombinedOutput()
	if err != nil {
		add("incremental_signed_helper", false)
		os.Stderr.Write(output)
		return
	}
	add("incremental_signed_helper", true)
	output, err = exec.CommandContext(ctx, "open", "-n", "-W", "-a", app, "--args", "setup-snapshot", home, evidence).CombinedOutput()
	add("setup_window_launched", err == nil)
	if err != nil {
		os.Stderr.Write(output)
	}
	data, err := os.ReadFile(filepath.Join(evidence, "report.json"))
	var report struct {
		StatusMatches   bool `json:"status_matches"`
		PermissionCount int  `json:"permission_count"`
		SnapshotCount   int  `json:"snapshot_count"`
		RequestCalls    int  `json:"request_calls"`
		FreshSetup      bool `json:"fresh_setup"`
		TCCMatches      bool `json:"tcc_matches"`
		TCCCompared     int  `json:"tcc_compared"`
	}
	add("snapshot_report", err == nil && json.Unmarshal(data, &report) == nil)
	add("fresh_setup_data_without_home_override", report.FreshSetup)
	add("no_permission_requests", report.RequestCalls == 0)
	add("live_native_status_stable", report.StatusMatches && report.PermissionCount >= 24)
	add("real_tcc_matches_checklist", report.TCCMatches && report.TCCCompared > 0)
	for step := 1; step <= 5; step++ {
		info, err := os.Stat(filepath.Join(evidence, "step-"+string(rune('0'+step))+".png"))
		add("step_"+string(rune('0'+step))+"_snapshot", err == nil && info.Size() > 1000)
	}
	add("five_steps_shown", report.SnapshotCount == 5)
	e.Metrics = map[string]any{"snapshot_count": report.SnapshotCount, "permission_count": report.PermissionCount, "request_calls": report.RequestCalls, "tcc_compared": report.TCCCompared}
	return
}
