package scenarios

import (
	"context"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func runRemindersStorePerf(ctx context.Context, s Scenario, level string) Evidence {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e := Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, GitSHA: strings.TrimSpace(string(sha)), At: time.Now().UTC(), Hosts: []string{"air"}, Latency: map[string]Latency{}}
	add := func(name string, passed bool) {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: passed})
	}
	if err := os.MkdirAll("/tmp/ablab", 0700); err != nil {
		add("private_profile_directory", false)
		return e
	}
	directory, err := os.MkdirTemp("/tmp/ablab", "reminders-store-perf-")
	add("private_profile_directory", err == nil)
	if err != nil {
		return e
	}
	defer os.RemoveAll(directory)
	report := filepath.Join(directory, "profile.json")
	command := exec.CommandContext(ctx, "packaging/dev/check-reminders-store.sh", "--perf-report", report)
	if level == "real" {
		app := filepath.Join(directory, "apps", "AppleBridgeHelper.app")
		output, buildErr := exec.CommandContext(ctx, "packaging/dev/build-helper.sh", app).CombinedOutput()
		add("private_signed_helper", buildErr == nil)
		if buildErr != nil {
			e.Assertions[len(e.Assertions)-1].Detail = string(output)
			return e
		}
		report = filepath.Join(os.Getenv("HOME"), "Library", "Application Support", "AppleBridgeDev", "w5-store-perf-"+e.RunID+".json")
		defer os.Remove(report)
		command = exec.CommandContext(ctx, "open", "-n", "-W", "-g", "-a", app, "--args", "reminders-store-profile", "--report", report)
		defer func() {
			processes, _ := exec.Command("ps", "-axo", "pid=,args=").Output()
			for _, line := range strings.Split(string(processes), "\n") {
				fields := strings.Fields(line)
				if len(fields) > 1 && fields[1] == filepath.Join(app, "Contents", "MacOS", "AppleBridgeHelper") {
					exec.Command("kill", fields[0]).Run()
				}
			}
		}()
	}
	output, err := command.CombinedOutput()
	add("store_profile_completed", err == nil)
	if err != nil {
		e.Assertions[len(e.Assertions)-1].Detail = string(output)
		return e
	}
	bytes, err := os.ReadFile(report)
	var profile map[string]json.RawMessage
	add("timing_only_profile_decoded", err == nil && json.Unmarshal(bytes, &profile) == nil)
	if len(profile) == 0 {
		return e
	}
	for _, mode := range []string{"warm_full", "warm_incremental_unchanged", "warm_incremental_changed_1", "warm_incremental_changed_10", "warm_incremental_changed_100", "warm_incremental_changed_1000"} {
		if profile[mode] == nil {
			continue
		}
		var components map[string]Latency
		if json.Unmarshal(profile[mode], &components) != nil {
			continue
		}
		e.Latency[mode] = components["total"]
		budget := 50.0
		if mode == "warm_full" {
			budget = 100
		}
		if mode == "warm_incremental_changed_1" || mode == "warm_incremental_changed_10" {
			budget = 5
		}
		add(mode+"_200_samples_within_budget", components["total"].Samples == 200 && components["total"].P95 < budget)
	}
	add("full_and_incremental_measured", e.Latency["warm_full"].Samples == 200 && e.Latency["warm_incremental_unchanged"].Samples == 200)
	if level == "sim" || level == "real" {
		complete := true
		for _, mode := range []string{"warm_incremental_changed_1", "warm_incremental_changed_10", "warm_incremental_changed_100", "warm_incremental_changed_1000"} {
			complete = complete && e.Latency[mode].Samples == 200
		}
		add("changed_history_measured", complete)
	}
	if level == "real" {
		var responsiveness struct {
			MainQueue struct {
				Under16 bool     `json:"under_16_ms"`
				Max     *float64 `json:"max_ms"`
				Samples int      `json:"samples"`
			} `json:"main_queue_delay"`
		}
		responsive := json.Unmarshal(profile["responsiveness"], &responsiveness) == nil && responsiveness.MainQueue.Samples > 0 && responsiveness.MainQueue.Under16 && responsiveness.MainQueue.Max != nil && *responsiveness.MainQueue.Max >= 0 && *responsiveness.MainQueue.Max < 16
		add("cold_full_read_main_queue_blocking_under_16_ms", responsive)
	}
	e.Metrics = map[string]any{"profile": profile}
	e.Passed = true
	for _, assertion := range e.Assertions {
		e.Passed = e.Passed && assertion.Passed
	}
	return e
}
