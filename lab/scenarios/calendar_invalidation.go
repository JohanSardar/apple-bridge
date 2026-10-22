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
	if s.ID == "CAL-STORE-NATIVE" {
		flag = "--store-reader"
	}
	command = exec.CommandContext(ctx, filepath.Join(strings.TrimSpace(string(directory)), "CalendarChecks"), flag)
	output, err = command.CombinedOutput()
	if s.ID == "CAL-HINT-COVERAGE" || s.ID == "CAL-STORE-NATIVE" {
		var checks map[string]bool
		decoded := json.Unmarshal(output, &checks) == nil
		add("native_hint_fixture_decoded", decoded)
		names := []string{"late_coalesced_hint_requires_followup", "all_hints_before_scan_are_skipped", "hint_at_scan_start_requires_followup"}
		if s.ID == "CAL-STORE-NATIVE" {
			names = []string{"store_blob_digest_and_bytes", "store_blob_scope_cannot_escape_calendar_root", "store_unverified_units_stay_hidden", "store_unique_identifier_join", "store_uuid_join", "store_external_identity_is_calendar_scoped", "store_unknown_calendar_rejects", "store_detached_identity_is_not_guessed", "store_fields_decoded_without_unit_guessing"}
		}
		for _, name := range names {
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
