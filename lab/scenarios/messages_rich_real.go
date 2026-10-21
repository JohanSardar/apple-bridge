package scenarios

import (
	"context"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"time"
)

const richParityLimit = "Signed dev.applebridge.truth lacks Full Disk Access to the Messages database. Granting access requires Johan's admin authentication in System Settings; identity-bound real parity is limited. No authorization is requested."

type richCountsReport struct {
	Status string         `json:"status"`
	Reason string         `json:"reason"`
	Counts map[string]int `json:"counts"`
}

func runRealRichMessages(ctx context.Context, s Scenario, e Evidence) Evidence {
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	if os.MkdirAll("/tmp/ablab", 0700) != nil {
		add("private_directory", false)
		return e
	}
	directory, err := os.MkdirTemp("/tmp/ablab", "w2-rich-")
	if err != nil {
		add("private_directory", false)
		return e
	}
	defer os.RemoveAll(directory)
	helper := filepath.Join(directory, "apps", "AppleBridgeHelper.app")
	truth := filepath.Join(directory, "apps", "AppleTruth.app")
	buildLog := filepath.Join("/tmp/ablab", "w2-rich-"+e.RunID+"-build.log")
	defer func() {
		if e.Passed {
			os.Remove(buildLog)
		}
	}()
	for _, build := range []struct{ script, destination string }{{"packaging/dev/build-helper.sh", helper}, {"packaging/dev/build-truth.sh", truth}} {
		command := exec.CommandContext(ctx, build.script, build.destination)
		log, err := os.OpenFile(buildLog, os.O_CREATE|os.O_APPEND|os.O_WRONLY, 0600)
		if err != nil {
			add("private_build_log", false)
			return e
		}
		command.Stdout = log
		command.Stderr = log
		err = command.Run()
		log.Close()
		add(filepath.Base(build.script), err == nil)
		if err != nil {
			return e
		}
	}
	report := filepath.Join(os.Getenv("HOME"), "Library", "Application Support", "AppleBridgeDev", "w2-rich-"+e.RunID+".json")
	defer os.Remove(report)
	start := time.Now()
	err = exec.CommandContext(ctx, "open", "-n", "-W", "-g", "-a", helper, "--args", "probe", "rich-counts", "--report", report).Run()
	data, readErr := os.ReadFile(report)
	var got richCountsReport
	add("signed_helper_readonly_counts", err == nil && readErr == nil && json.Unmarshal(data, &got) == nil && got.Status == "verified")
	if got.Counts == nil {
		return e
	}
	e.Metrics["helper_counts"] = got.Counts
	e.Latency["readonly_helper_decoding"] = stats([]float64{float64(time.Since(start)) / float64(time.Millisecond)}, 0)
	input := filepath.Join(directory, "truth-request.json")
	output := filepath.Join(directory, "truth-report.json")
	request, _ := json.Marshal(map[string]any{"source": "messages", "rich_counts": true})
	if os.WriteFile(input, request, 0600) != nil {
		add("private_truth_request", false)
		return e
	}
	err = exec.CommandContext(ctx, "open", "-n", "-W", "-g", "-a", truth, "--args", "observe", "--request-file", input, "--report", output).Run()
	data, readErr = os.ReadFile(output)
	var want richCountsReport
	add("independent_apple_truth_readonly_counts", err == nil && readErr == nil && json.Unmarshal(data, &want) == nil && want.Status == "verified")
	if want.Reason == "permissionMissing" {
		e.Status = "limited"
		e.Assertions[len(e.Assertions)-1].Status = "unverified"
		e.Assertions[len(e.Assertions)-1].Detail = richParityLimit
		e.Metrics["limits"] = []map[string]string{{"scenario": s.ID, "level": "real", "reason": richParityLimit}}
	}
	if want.Counts == nil {
		return e
	}
	for key, count := range want.Counts {
		add("counts_match_"+key, got.Counts[key] == count)
	}
	add("every_count_has_independent_truth", reflect.DeepEqual(got.Counts, want.Counts))
	e.Metrics["truth_counts"] = want.Counts
	e.Passed = true
	for _, a := range e.Assertions {
		e.Passed = e.Passed && a.Passed
	}
	return e
}
