package scenarios

import (
	"context"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func runMediaExportSafety(ctx context.Context, s Scenario, level string) Evidence {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e := Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, GitSHA: strings.TrimSpace(string(sha)), At: time.Now().UTC(), Hosts: []string{"air"}, Latency: map[string]Latency{}, Metrics: map[string]any{"synthetic_only": true}}
	if level == "real" {
		return runRealMediaExportSafety(ctx, e)
	}
	directory := filepath.Join("lab", "evidence", "private", "export-safety")
	os.MkdirAll(directory, 0o700)
	for _, check := range []struct{ command, success string }{
		{"export-safety", "synthetic incompatible export rejected; following export completed"},
		{"export-overlap", "same-digest work serialized across processes; cached result reused"},
	} {
		output, err := exec.CommandContext(ctx, mediaTool(), check.command).CombinedOutput()
		os.WriteFile(filepath.Join(directory, e.RunID+"-"+check.command+".log"), output, 0o600)
		assertion := Assertion{Name: check.command, Passed: err == nil && strings.Contains(string(output), check.success)}
		if err != nil {
			assertion.Detail = "synthetic native export check: " + err.Error()
		}
		e.Assertions = append(e.Assertions, assertion)
	}
	e.Passed = true
	for _, assertion := range e.Assertions {
		e.Passed = e.Passed && assertion.Passed
	}
	return e
}
