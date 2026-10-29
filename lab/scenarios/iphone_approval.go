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

func runIPhoneApproval(ctx context.Context, s Scenario, level string) Evidence {
	e := Evidence{Scenario: s.ID, Level: level, At: time.Now().UTC(), Passed: true}
	add := func(name string, ok bool) {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok})
		e.Passed = e.Passed && ok
	}
	dir, err := os.MkdirTemp("", "ab-test-approval-")
	add("temporary_binary", err == nil)
	if err != nil {
		return e
	}
	defer os.RemoveAll(dir)
	binary := filepath.Join(dir, "checks")
	cmd := exec.CommandContext(ctx, "xcrun", "swiftc", "-parse-as-library", "ios/Capabilities/AppleApproval/ApprovalService.swift", "lab/ios/ApprovalChecks.swift", "-o", binary)
	cmd.Env = append(os.Environ(), "SDKROOT=")
	buildOutput, buildErr := cmd.CombinedOutput()
	add("swift_adapter_build", buildErr == nil)
	if buildErr != nil {
		os.Stderr.Write(buildOutput)
		return e
	}
	out, err := exec.CommandContext(ctx, binary).Output()
	add("swift_flags_and_fake_adapters", err == nil && strings.TrimSpace(string(out)) == "PASS")
	for _, item := range []struct{ op, args string }{
		{"iphone.wallet.account", `{"device_id":"iphone:test","limit":10,"offset":0}`},
		{"iphone.wallet.transaction", `{"device_id":"iphone:test"}`},
		{"iphone.critical.alert", `{"device_id":"iphone:test","title":"Bridge Test","body":"Bridge Test"}`},
		{"iphone.location.push", `{"device_id":"iphone:test"}`},
		{"iphone.fall.detection", `{"device_id":"iphone:test"}`},
	} {
		add(item.op+"_gateway_admission", contracts.ValidateIPhoneCommand(contracts.Command{ID: "cmd_approval", Op: item.op, Args: []byte(item.args)}) == nil)
	}
	add("wallet_limit_rejected", contracts.ValidateIPhoneCommand(contracts.Command{ID: "cmd_approval", Op: "iphone.wallet.account", Args: []byte(`{"device_id":"iphone:test","limit":51}`)}) != nil)
	add("empty_critical_alert_rejected", contracts.ValidateIPhoneCommand(contracts.Command{ID: "cmd_approval", Op: "iphone.critical.alert", Args: []byte(`{"device_id":"iphone:test","title":""}`)}) != nil)
	return e
}
