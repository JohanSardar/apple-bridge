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

func runMediaYield(ctx context.Context, s Scenario, level string) Evidence {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	directory := filepath.Join("lab", "evidence", "private", "media-yield")
	os.MkdirAll(directory, 0o700)
	e := Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, GitSHA: strings.TrimSpace(string(sha)), At: time.Now().UTC(), Hosts: []string{"air"}, Latency: map[string]Latency{}, Metrics: map[string]any{"synthetic_only": true}, Passed: true}
	for _, check := range []struct{ command, name, success string }{
		{"--media-yield-burst", "foreground_burst_reuses_completed_stages", "cooperative media burst checks passed"},
		{"--media-yield-metadata", "metadata_only_changes_preserve_retained_stages", "cooperative media burst checks passed"},
		{"--media-yield-source-change", "changed_source_invalidates_retained_stages", "cooperative media burst checks passed"},
		{"--media-yield-companion", "local_companion_wakes_deferred_live_photo", "cooperative media burst checks passed"},
		{"--media-yield-export-race", "export_yield_retries_without_cancelling_parent", "cooperative media export checks passed"},
	} {
		output, err := exec.CommandContext(ctx, filepath.Join(filepath.Dir(mediaTool()), "MessagesCaptureChecks"), check.command).CombinedOutput()
		os.WriteFile(filepath.Join(directory, e.RunID+check.command+".log"), output, 0o600)
		passed := err == nil && strings.Contains(string(output), check.success)
		assertion := Assertion{Name: check.name, Passed: passed}
		if err != nil {
			assertion.Detail = "native cooperative yield check: " + err.Error()
		}
		for _, line := range strings.Split(string(output), "\n") {
			var metrics map[string]any
			if json.Unmarshal([]byte(line), &metrics) == nil && metrics["maximum_starts_per_digest"] != nil {
				e.Metrics[check.name] = metrics
			}
		}
		e.Assertions = append(e.Assertions, assertion)
		e.Passed = e.Passed && passed
	}
	return e
}
