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

func runIPhoneScreenTime(ctx context.Context, s Scenario, level string) Evidence {
	e := Evidence{Scenario: s.ID, Level: level, At: time.Now().UTC(), Passed: true}
	if level == "real" {
		e.Passed = false
		e.Status = "limited"
		e.Assertions = []Assertion{{Name: "authorization", Passed: false, Detail: "limited: needs Allow tap"}}
		return e
	}
	add := func(name string, ok bool) {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok})
		e.Passed = e.Passed && ok
	}
	dir, err := os.MkdirTemp("", "ab-test-screentime-")
	add("isolated_fixture", err == nil)
	if err != nil {
		return e
	}
	defer os.RemoveAll(dir)
	binary := filepath.Join(dir, "shield-policy")
	err = exec.CommandContext(ctx, "swiftc", "ios/ScreenTimeShared/ShieldPolicy.swift", "lab/ios/main.swift", "-o", binary).Run()
	add("swift_policy_compiled", err == nil)
	if err == nil {
		output, err := exec.CommandContext(ctx, binary).Output()
		add("fake_managed_settings_shield_unshield_downtime_isolation", err == nil && strings.TrimSpace(string(output)) == "shield_policy_pass")
	}
	for _, op := range []string{"shield", "unshield", "cancel", "schedule"} {
		raw := []byte(`{"device_id":"iphone:test"}`)
		if op == "schedule" {
			raw = []byte(`{"device_id":"iphone:test","start_minute":1380,"end_minute":60}`)
		}
		add("gateway_"+op, contracts.ValidateIPhoneCommand(contracts.Command{ID: "cmd_screentime", Op: "iphone.screentime." + op, Args: raw}) == nil)
	}
	add("invalid_schedule_rejected", contracts.ValidateIPhoneCommand(contracts.Command{ID: "cmd_screentime", Op: "iphone.screentime.schedule", Args: []byte(`{"device_id":"iphone:test","start_minute":60,"end_minute":61}`)}) != nil)
	return e
}
