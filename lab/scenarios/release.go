package scenarios

import (
	"context"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"
)

func runRelease(ctx context.Context, level string, evidence Evidence) (result Evidence) {
	result = evidence
	result.Hosts = []string{"pro"}
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
	assessment, err := exec.CommandContext(ctx, "spctl", "-a", "-vv", "--type", "open", "--context", "context:primary-signature", dmg).CombinedOutput()
	add("gatekeeper_accepts_dmg", err == nil && strings.Contains(string(assessment), "accepted") && strings.Contains(string(assessment), "source=Notarized Developer ID"))
	mount, err := os.MkdirTemp("", "ab-test-release-")
	add("private_mount_directory", err == nil)
	if err != nil {
		return result
	}
	defer os.Remove(mount)
	err = exec.CommandContext(ctx, "hdiutil", "attach", "-readonly", "-nobrowse", "-mountpoint", mount, dmg).Run()
	add("readonly_dmg_mount", err == nil)
	if err != nil {
		return result
	}
	defer func() {
		cleanup, cancel := context.WithTimeout(context.Background(), 30*time.Second)
		defer cancel()
		add("dmg_unmounted", exec.CommandContext(cleanup, "hdiutil", "detach", mount).Run() == nil)
	}()
	app := filepath.Join(mount, "Apple Bridge.app")
	add("bundled_app_signature", exec.CommandContext(ctx, "codesign", "--verify", "--deep", "--strict", app).Run() == nil)
	add("bundled_app_stapled", exec.CommandContext(ctx, "xcrun", "stapler", "validate", app).Run() == nil)
	applications, err := os.Readlink(filepath.Join(mount, "Applications"))
	add("drag_to_applications", err == nil && applications == "/Applications")
	return result
}
