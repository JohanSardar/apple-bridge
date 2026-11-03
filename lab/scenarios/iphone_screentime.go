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

func runIPhoneScreenTime(ctx context.Context, s Scenario, level string) Evidence {
	e := Evidence{Scenario: s.ID, Level: level, At: time.Now().UTC(), Passed: true}
	if level == "real" {
		e.Passed = false
		e.Status = "limited"
		e.Assertions = []Assertion{{Name: "authorization", Passed: false, Detail: "limited: real-device permission request and observation harness missing; needs Allow tap"}}
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
	err = iphoneProjectCommand(ctx, "swiftc", "ios/ScreenTimeShared/ShieldPolicy.swift", "lab/ios/main.swift", "-o", binary).Run()
	add("swift_policy_compiled", err == nil)
	if err == nil {
		output, err := exec.CommandContext(ctx, binary).Output()
		add("fake_managed_settings_shield_unshield_downtime_isolation", err == nil && strings.TrimSpace(string(output)) == "shield_policy_pass")
	}

	gateway, phone, token, err := iphoneBinaryFixture(ctx, dir)
	add("binary_gateway_fixture", err == nil)
	if err != nil {
		return e
	}
	defer gateway.Close()
	for _, op := range []string{"shield", "unshield", "cancel", "schedule"} {
		args := map[string]any{"device_id": phone.ID}
		if op == "schedule" {
			args["start_minute"], args["end_minute"] = 1380, 60
		}
		raw, _ := json.Marshal(args)
		var admitted contracts.CommandRecord
		err := gateway.Invoke("iphone.screentime."+op, contracts.Command{ID: "cmd_screentime_" + op, Args: raw}, token, &admitted)
		add("gateway_"+op, err == nil && admitted.State == contracts.CommandAdmitted)
	}
	raw, _ := json.Marshal(map[string]any{"device_id": phone.ID, "start_minute": 60, "end_minute": 61})
	status, err := gateway.Request("/v1/op/iphone.screentime.schedule", contracts.Command{ID: "cmd_screentime_invalid", Args: raw}, token, nil)
	add("invalid_schedule_rejected", err == nil && status == 400)
	return e
}
