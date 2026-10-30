package scenarios

import (
	"context"
	"os"
	"os/exec"
	"path/filepath"
	"time"
)

func runIPhoneAlarm(ctx context.Context, s Scenario, level string) Evidence {
	e := Evidence{Scenario: s.ID, Level: level, At: time.Now().UTC()}
	if level == "real" {
		e.Status = "limited"
		e.Assertions = []Assertion{{Name: "alarmkit_permission", Detail: "limited: real-device permission request and observation harness missing; needs Allow tap"}}
		return e
	}
	dir, err := os.MkdirTemp("", "ab-test-alarms-")
	if err != nil {
		return e
	}
	defer os.RemoveAll(dir)
	binary := filepath.Join(dir, "checks")
	err = exec.CommandContext(ctx, "swiftc", "-parse-as-library", "ios/Capabilities/Alarms/AlarmCommands.swift", "lab/iphonealarms/main.swift", "-o", binary).Run()
	e.Assertions = append(e.Assertions, Assertion{Name: "swift_handler_build", Passed: err == nil})
	if err == nil {
		err = exec.CommandContext(ctx, binary).Run()
		e.Assertions = append(e.Assertions, Assertion{Name: "alarm_timer_lifecycle_and_permission", Passed: err == nil})
	}
	e.Passed = err == nil
	return e
}
