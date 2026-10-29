package scenarios

import (
	"context"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"

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
	report := filepath.Join(os.Getenv("HOME"), "Library", "Application Support", "AppleBridgeDev", "photos-"+e.RunID+".json")
	defer os.Remove(report)
	out, err = exec.CommandContext(ctx, "open", "-n", "-W", "-a", app, "--args", "photos-real-checks", s.ID, "--report", report).CombinedOutput()
	add("signed_helper_probe", err == nil)
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
	for _, name := range []string{"asset_count", "original_digest"} {
		e.Metrics[name] = result[name]
	}
	if s.ID == "PHO-IDLE" && e.Passed {
		return wholeHelperIdleReal(world.WithOptions(ctx, world.Options{Scope: "photos:read,system:read"}), e)
	}
	return e
}
