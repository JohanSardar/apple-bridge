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

func runCapture(ctx context.Context, s Scenario, level string) Evidence {
	e := Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, At: time.Now().UTC(), Passed: true}
	add := func(name string, ok bool) {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok})
		e.Passed = e.Passed && ok
	}
	directory, directoryErr := os.ReadFile("native/.build/native-output-1")
	fresh := directoryErr == nil && captureChecksFresh(filepath.Join(strings.TrimSpace(string(directory)), "CaptureChecks"))
	var out []byte
	var err error
	if !fresh {
		build := exec.CommandContext(ctx, "packaging/dev/build-native.sh")
		build.Env = append(os.Environ(), "APPLE_BRIDGE_NATIVE_TARGET=CaptureChecks")
		out, err = build.CombinedOutput()
		add("native_build", err == nil)
		if err != nil {
			os.Stderr.Write(out)
			return e
		}
	}
	directory, err = os.ReadFile("native/.build/native-output-1")
	add("native_output", err == nil)
	if err != nil {
		return e
	}
	out, err = exec.CommandContext(ctx, filepath.Join(strings.TrimSpace(string(directory)), "CaptureChecks")).CombinedOutput()
	add("provider_lifecycle_and_artifact_verification", err == nil)
	if err != nil {
		os.Stderr.Write(out)
	}
	return e
}

func captureChecksFresh(binary string) bool {
	info, err := os.Stat(binary)
	if err != nil {
		return false
	}
	fresh := true
	for _, root := range []string{"native/Package.swift", "native/Sources/CaptureSource", "native/Sources/CaptureChecks", "native/Sources/SourceSupport"} {
		err := filepath.Walk(root, func(_ string, item os.FileInfo, err error) error {
			if err != nil {
				return err
			}
			if item.ModTime().After(info.ModTime()) {
				fresh = false
			}
			return nil
		})
		if err != nil {
			return false
		}
	}
	return fresh
}
