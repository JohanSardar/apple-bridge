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

func runSourceSupport(ctx context.Context, s Scenario, level string) Evidence {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e := Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, GitSHA: strings.TrimSpace(string(sha)), At: time.Now().UTC(), Hosts: []string{"air"}, Latency: map[string]Latency{}}
	cmd := exec.CommandContext(ctx, "packaging/dev/build-native.sh")
	cmd.Env = append(os.Environ(), "APPLE_BRIDGE_NATIVE_TARGET=SourceSupportChecks")
	_, err := cmd.CombinedOutput()
	e.Assertions = append(e.Assertions, Assertion{Name: "native_support_checks_build", Passed: err == nil})
	if err == nil {
		directory, readErr := os.ReadFile("native/.build/native-output-1")
		if readErr == nil {
			output, runErr := exec.CommandContext(ctx, filepath.Join(strings.TrimSpace(string(directory)), "SourceSupportChecks")).CombinedOutput()
			e.Passed = runErr == nil && strings.TrimSpace(string(output)) == "all SourceSupport checks passed"
		}
	}
	e.Assertions = append(e.Assertions, Assertion{Name: "generation_wal_complete_scan_blob_checks", Passed: e.Passed})
	return e
}
