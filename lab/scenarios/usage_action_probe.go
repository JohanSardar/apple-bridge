package scenarios

import (
	"context"
	"os"
	"os/exec"
	"path/filepath"
)

func runUsageActionProbe(ctx context.Context, s Scenario, level string) Evidence {
	e := runUsageData(ctx, s, level)
	if level == "sim" {
		directory, err := os.MkdirTemp("", "ab-usage-truth-checks-")
		passed := err == nil
		if passed {
			defer os.RemoveAll(directory)
			executable := filepath.Join(directory, "UsageTruthChecks")
			output, err := exec.CommandContext(ctx, "xcrun", "swiftc", "-swift-version", "6", "-warnings-as-errors", "-parse-as-library", "lab/truth/native/Sources/AppleTruth/Usage.swift", "lab/fixtures/usage/probe-truth-checks.swift", "-o", executable).CombinedOutput()
			passed = err == nil
			if !passed {
				os.Stderr.Write(output)
			} else {
				output, err = exec.CommandContext(ctx, executable).CombinedOutput()
				passed = err == nil
				if !passed {
					os.Stderr.Write(output)
				}
			}
		}
		e.Assertions = append(e.Assertions, Assertion{Name: "independent_truth_handles_retired_peers", Passed: passed})
		e.Passed = e.Passed && passed
	}
	return e
}
