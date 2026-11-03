package scenarios

import (
	"context"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"
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
	cmd := iphoneProjectCommand(ctx, "xcrun", "swiftc", "-parse-as-library", "ios/Capabilities/HealthWrite/HealthLog.swift", "lab/ios/HealthWriteProof.swift", "-o", binary)
	output, err := cmd.CombinedOutput()
	e.Assertions = append(e.Assertions, Assertion{Name: "swift_write_build", Passed: err == nil})
	if err != nil {
		return e
	}
	output, err = exec.CommandContext(ctx, binary).Output()
	e.Passed = err == nil && strings.TrimSpace(string(output)) == "PASS"
	e.Assertions = append(e.Assertions, Assertion{Name: "four_tagged_samples_written_read_deleted", Passed: e.Passed})
	valid := false
	delivered := proveIPhoneCapabilityDelivery(ctx, dir, []byte(`{"id":"ab-test"}`), []string{"iphone.health.log"}, func(ok bool) { valid = ok })
	e.Assertions = append(e.Assertions, Assertion{Name: "gateway_admission", Passed: valid})
	e.Passed = e.Passed && valid
	e.Assertions = append(e.Assertions, Assertion{Name: "signed_health_command_roundtrip", Passed: delivered})
	e.Passed = e.Passed && delivered
	return e
}
