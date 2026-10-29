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

func runIPhoneExtras(ctx context.Context, s Scenario, level string) Evidence {
	e := Evidence{Scenario: s.ID, Level: level, At: time.Now().UTC(), Passed: true}
	add := func(name string, ok bool) {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok})
		e.Passed = e.Passed && ok
	}
	dir, err := os.MkdirTemp("", "ab-test-extras-")
	add("temporary_binary", err == nil)
	if err != nil {
		return e
	}
	defer os.RemoveAll(dir)
	binary := filepath.Join(dir, "checks")
	cmd := exec.CommandContext(ctx, "xcrun", "swiftc", "-parse-as-library", "ios/Capabilities/Extras/ExtrasService.swift", "lab/ios/ExtrasChecks.swift", "-o", binary)
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
		{"iphone.weather", `{"device_id":"iphone:test","latitude":10,"longitude":20}`},
		{"iphone.journal.suggestions", `{"device_id":"iphone:test"}`},
		{"iphone.energy.hints", `{"device_id":"iphone:test","venue_id":"00000000-0000-0000-0000-000000000001"}`},
	} {
		add(item.op+"_gateway_admission", contracts.ValidateIPhoneCommand(contracts.Command{ID: "cmd_extras", Op: item.op, Args: []byte(item.args)}) == nil)
	}
	return e
}
