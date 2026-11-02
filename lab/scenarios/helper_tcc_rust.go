package scenarios

import (
	"context"
	"encoding/json"
	"os"
	"os/exec"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func runRustTCCReady(ctx context.Context, s Scenario, level string) Evidence {
	sha, _ := exec.CommandContext(ctx, "git", "rev-parse", "HEAD").Output()
	e := Evidence{Impl: "rust", RunID: contracts.NewID("run"), GitSHA: strings.TrimSpace(string(sha)), Scenario: s.ID, Level: level, At: time.Now().UTC(), Hosts: []string{"air"}}
	if level != "real" {
		e.Status = "unverified"
		return e
	}
	file, err := os.CreateTemp("/tmp", "ab-tcc-evidence-*.json")
	if err != nil {
		return e
	}
	path := file.Name()
	file.Close()
	defer os.Remove(path)
	command := exec.CommandContext(ctx, "python3", "rust/lab/identity-spikes/run.py", path, "--tcc-only")
	output, err := command.CombinedOutput()
	if err != nil {
		e.Assertions = append(e.Assertions, Assertion{Name: "rust_probe_run", Passed: false, Detail: string(output)})
		return e
	}
	data, err := os.ReadFile(path)
	var results map[string]json.RawMessage
	if err != nil || json.Unmarshal(data, &results) != nil {
		return e
	}
	e.Metrics = map[string]any{"rust_identity": json.RawMessage(data)}
	e.Passed = true
	for _, mode := range []string{"open", "resigned", "control"} {
		var reads map[string]json.RawMessage
		if json.Unmarshal(results[mode], &reads) != nil {
			e.Passed = false
			continue
		}
		for _, source := range []string{"chat", "wal", "attachment", "notes", "biome", "safari"} {
			var read struct {
				Pass bool `json:"pass"`
			}
			raw, exists := reads[source]
			valid := exists && json.Unmarshal(raw, &read) == nil
			passed := valid && read.Pass
			if mode == "control" {
				passed = valid && !read.Pass
			}
			e.Assertions = append(e.Assertions, Assertion{Name: mode + "_" + source, Passed: passed})
			e.Passed = e.Passed && passed
		}
	}
	var cleaned bool
	json.Unmarshal(results["control_cleaned"], &cleaned)
	e.Assertions = append(e.Assertions, Assertion{Name: "launchd_control_removed", Passed: cleaned})
	e.Passed = e.Passed && cleaned
	if !e.Passed {
		e.Status = "unverified"
	}
	return e
}
