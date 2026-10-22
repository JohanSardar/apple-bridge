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

func runCalendarInvalidation(ctx context.Context, s Scenario, level string) Evidence {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e := Evidence{GitSHA: strings.TrimSpace(string(sha)), RunID: contracts.NewID("cal"), Scenario: s.ID, Level: level, At: time.Now().UTC(), Conditions: calendarHostConditions(), Hosts: []string{"air"}, Passed: true, Latency: map[string]Latency{}}
	add := func(name string, ok bool) {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok})
		e.Passed = e.Passed && ok
	}
	command := exec.CommandContext(ctx, "packaging/dev/build-native.sh")
	command.Env = append(os.Environ(), "APPLE_BRIDGE_NATIVE_TARGET=CalendarChecks", "APPLE_BRIDGE_NATIVE_LAB=1")
	output, err := command.CombinedOutput()
	add("native_calendar_checks_build", err == nil)
	if err != nil {
		os.Stderr.Write(output)
		return e
	}
	directory, err := os.ReadFile("native/.build/native-output-1")
	add("native_checks_output_available", err == nil)
	if err != nil {
		return e
	}
	flag := "--invalidated-event"
	if s.ID == "CAL-HINT-COVERAGE" {
		flag = "--hint-coverage"
	}
	command = exec.CommandContext(ctx, filepath.Join(strings.TrimSpace(string(directory)), "CalendarChecks"), flag)
	output, err = command.CombinedOutput()
	if s.ID == "CAL-HINT-COVERAGE" {
		var checks map[string]bool
		decoded := json.Unmarshal(output, &checks) == nil
		add("native_hint_fixture_decoded", decoded)
		for _, name := range []string{"late_coalesced_hint_requires_followup", "all_hints_before_scan_are_skipped", "hint_at_scan_start_requires_followup"} {
			add(name, decoded && checks[name])
		}
		if !decoded {
			os.Stderr.Write(output)
		}
		return e
	}
	add("invalidated_calendar_object_rebuilds_without_trap", err == nil)
	if err != nil {
		os.Stderr.Write(output)
	}
	return e
}
