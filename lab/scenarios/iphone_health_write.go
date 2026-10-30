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

func runIPhoneHealthWrite(ctx context.Context, s Scenario, level string) Evidence {
	e := Evidence{Scenario: s.ID, Level: level, At: time.Now().UTC()}
	if level == "real" {
		e.Status = "limited"
		e.Assertions = []Assertion{{Name: "health_write_permission", Detail: "limited: real-device permission request and observation harness missing; needs Allow tap"}}
		return e
	}
	dir, err := os.MkdirTemp("", "ab-test-health-write-")
	if err != nil {
		return e
	}
	defer os.RemoveAll(dir)
	binary := filepath.Join(dir, "proof")
	cmd := exec.CommandContext(ctx, "xcrun", "swiftc", "-parse-as-library", "ios/Capabilities/HealthWrite/HealthLog.swift", "lab/ios/HealthWriteProof.swift", "-o", binary)
	cmd.Env = append(os.Environ(), "SDKROOT=")
	output, err := cmd.CombinedOutput()
	e.Assertions = append(e.Assertions, Assertion{Name: "swift_write_build", Passed: err == nil})
	if err != nil {
		return e
	}
	output, err = exec.CommandContext(ctx, binary).Output()
	e.Passed = err == nil && strings.TrimSpace(string(output)) == "PASS"
	e.Assertions = append(e.Assertions, Assertion{Name: "four_tagged_samples_written_read_deleted", Passed: e.Passed})
	valid := contracts.ValidateIPhoneCommand(contracts.Command{ID: "cmd_health_write", Op: "iphone.health.log", Args: []byte(`{"device_id":"iphone:ab-test","kind":"water","value":250,"start":"2026-10-29T12:00:00Z","tag":"[ab-test]"}`)}) == nil
	e.Assertions = append(e.Assertions, Assertion{Name: "gateway_admission", Passed: valid})
	e.Passed = e.Passed && valid
	delivered := proveIPhoneCapabilityDelivery(ctx, dir, []byte(`{"id":"ab-test"}`), []string{"iphone.health.log"})
	e.Assertions = append(e.Assertions, Assertion{Name: "signed_health_command_roundtrip", Passed: delivered})
	e.Passed = e.Passed && delivered
	return e
}
