package scenarios

import (
	"context"
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"time"

	"github.com/JohanSardar/apple-bridge/lab/world"
)

func runPhotosReal(ctx context.Context, s Scenario, e Evidence) Evidence {
	add := func(name string, ok bool) {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok})
		e.Passed = e.Passed && ok
	}
	dir, err := os.MkdirTemp("", "ab-photos-real-")
	add("helper_directory", err == nil)
	if err != nil {
		return e
	}
	defer os.RemoveAll(dir)
	app := filepath.Join(dir, "AppleBridgeHelper.app")
	build := exec.CommandContext(ctx, "packaging/dev/build-helper.sh", app)
	out, err := build.CombinedOutput()
	add("signed_helper_build", err == nil)
	if err != nil {
		os.Stderr.Write(out)
		return e
	}
	report := filepath.Join(os.Getenv("HOME"), "Library", "Application Support", "AppleBridgeDev", "photos-real-shared.json")
	binary, binaryErr := os.ReadFile(filepath.Join(app, "Contents", "MacOS", "AppleBridgeHelper"))
	add("helper_identity", binaryErr == nil)
	if binaryErr != nil {
		return e
	}
	identity := fmt.Sprintf("%x", sha256.Sum256(binary))
	cachedIdentity, _ := os.ReadFile(report + ".sha256")
	cached, _ := os.ReadFile(report)
	if info, statErr := os.Stat(report); statErr == nil && s.ID != "PHO-IDLE" && time.Since(info.ModTime()) < 15*time.Minute && string(cachedIdentity) == identity && photosReportPassed(cached) {
		add("signed_helper_probe", true)
	} else {
		os.Remove(report)
		out, err = exec.CommandContext(ctx, "open", "-n", "-W", "-a", app, "--args", "photos-real-checks", s.ID, "--report", report).CombinedOutput()
		add("signed_helper_probe", err == nil)
		if err == nil {
			err = os.WriteFile(report+".sha256", []byte(identity), 0600)
		}
	}
	if err != nil {
		os.Stderr.Write(out)
		return e
	}
	data, err := os.ReadFile(report)
	var result map[string]any
	if err == nil {
		err = json.Unmarshal(data, &result)
	}
	add("helper_report", err == nil)
	if failure, ok := result["failure"].(string); ok {
		e.Metrics["library_asset_count"] = result["library_asset_count"]
		e.Metrics["cleanup_passed"] = result["cleanup_passed"]
		e.Passed = false
		e.Assertions = append(e.Assertions, Assertion{Name: "native_probe_completed", Passed: false, Detail: failure})
		return e
	}
	if err != nil {
		return e
	}
	granted, known := result["grant_passed"].(bool)
	add("grant_probe", known)
	if !known {
		return e
	}
	if !granted {
		e.Passed = false
		e.Status = "limited"
		e.Assertions = append(e.Assertions, Assertion{Name: "photos_readwrite_authorization", Status: "limited", Detail: "needs Photos grant"})
		return e
	}
	for _, name := range []string{"read_passed", "import_passed", "original_passed", "cleanup_passed"} {
		add(name, result[name] == true)
	}
	for _, name := range []string{"asset_count", "original_digest", "library_asset_count"} {
		e.Metrics[name] = result[name]
	}
	if s.ID == "PHO-IDLE" && e.Passed {
		return wholeHelperIdleReal(world.WithOptions(ctx, world.Options{Scope: "photos:read,system:read"}), e)
	}
	return e
}

func photosReportPassed(data []byte) bool {
	var result map[string]any
	if json.Unmarshal(data, &result) != nil || result["failure"] != nil {
		return false
	}
	for _, name := range []string{"grant_passed", "read_passed", "import_passed", "original_passed", "cleanup_passed"} {
		if result[name] != true {
			return false
		}
	}
	return true
}
