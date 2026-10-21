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

func runCalendarInvalidation(ctx context.Context, s Scenario, level string) Evidence {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e := Evidence{GitSHA: strings.TrimSpace(string(sha)), RunID: contracts.NewID("cal"), Scenario: s.ID, Level: level, At: time.Now().UTC(), Conditions: calendarHostConditions(), Hosts: []string{"air"}, Passed: true, Latency: map[string]Latency{}}
	add := func(name string, ok bool) {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok})
		e.Passed = e.Passed && ok
	}
	command := exec.CommandContext(ctx, "packaging/dev/build-native.sh")
	command.Env = append(os.Environ(), "APPLE_BRIDGE_NATIVE_TARGET=CalendarChecks")
	output, err := command.CombinedOutput()
	add("native_calendar_checks_build", err == nil)
	if err != nil {
		os.Stderr.Write(output)
		return e
	}
	directory, err := os.ReadFile("native/.build/native-output")
	add("native_checks_output_available", err == nil)
	if err != nil {
		return e
	}
	command = exec.CommandContext(ctx, filepath.Join(strings.TrimSpace(string(directory)), "CalendarChecks"), "--invalidated-event")
	output, err = command.CombinedOutput()
	add("invalidated_calendar_object_rebuilds_without_trap", err == nil)
	if err != nil {
		os.Stderr.Write(output)
	}
	return e
}
