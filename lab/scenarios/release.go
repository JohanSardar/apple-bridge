package scenarios

import (
	"context"
	"os"
	"os/exec"
	"strings"
)

func runRelease(ctx context.Context, level string, evidence Evidence) (result Evidence) {
	result = evidence
	defer func() {
		result.Passed = len(result.Assertions) > 0
		for _, assertion := range result.Assertions {
			result.Passed = result.Passed && assertion.Passed
		}
	}()
	add := func(name string, ok bool) {
		result.Assertions = append(result.Assertions, Assertion{Name: name, Passed: ok})
	}
	if level == "sim" {
		output, err := exec.CommandContext(ctx, "packaging/release/check.sh").CombinedOutput()
		add("release_unit_checks", err == nil)
		for _, id := range []string{"APPCAST", "VERSION", "SIGNATURE", "UNINSTALL"} {
			add(strings.ToLower(id), strings.Contains(string(output), "R1-"+id+" passed=true"))
		}
		if err != nil {
			os.Stderr.Write(output)
		}
		return result
	}
	dmg := os.Getenv("APPLE_BRIDGE_RELEASE_DMG")
	add("local_notarized_dmg_available", dmg != "")
	if dmg == "" {
		return result
	}
	add("stapled_dmg", exec.CommandContext(ctx, "xcrun", "stapler", "validate", dmg).Run() == nil)
	add("gatekeeper_accepts_dmg", exec.CommandContext(ctx, "spctl", "-a", "-vv", "--type", "open", "--context", "context:primary-signature", dmg).Run() == nil)
	return result
}
