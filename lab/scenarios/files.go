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

func runFiles(ctx context.Context, s Scenario, level string) Evidence {
	if s.ID == "FILE-COMMANDS" || level == "sim" && (s.ID == "FILE-OFFLOAD-REAL" || s.ID == "FILE-CRUD-REAL") {
		return runFileCommands(ctx, s)
	}
	if s.ID == "FILE-REPLICA" {
		return runFilesReplica(ctx, s)
	}
	e := Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, At: time.Now().UTC(), Passed: true, Metrics: map[string]any{}}
	add := func(name string, ok bool) {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok})
		e.Passed = e.Passed && ok
	}
	directory, err := os.ReadFile("native/.build/native-output-1")
	binary := filepath.Join(strings.TrimSpace(string(directory)), "FilesChecks")
	info, statErr := os.Stat(binary)
	fresh := err == nil && statErr == nil
	if fresh {
		for _, root := range []string{"native/Package.swift", "native/Sources/FilesSource", "native/Sources/FilesChecks", "native/Sources/SourceSupport"} {
			walkErr := filepath.Walk(root, func(_ string, item os.FileInfo, walkErr error) error {
				if walkErr != nil {
					return walkErr
				}
				if item.ModTime().After(info.ModTime()) {
					fresh = false
				}
				return nil
			})
			fresh = fresh && walkErr == nil
		}
	}
	if !fresh {
		build := exec.CommandContext(ctx, "packaging/dev/build-native.sh")
		build.Env = append(os.Environ(), "APPLE_BRIDGE_NATIVE_TARGET=FilesChecks")
		out, buildErr := build.CombinedOutput()
		add("native_build", buildErr == nil)
		if buildErr != nil {
			os.Stderr.Write(out)
			return e
		}
		directory, err = os.ReadFile("native/.build/native-output-1")
		add("native_output", err == nil)
		if err != nil {
			return e
		}
	}
	args := []string{s.ID}
	if s.ID == "FILE-IDLE" && level == "real" {
		args = append(args, "--real-idle")
	}
	check := exec.CommandContext(ctx, filepath.Join(strings.TrimSpace(string(directory)), "FilesChecks"), args...)
	out, err := check.CombinedOutput()
	add("native_files_checks", err == nil)
	for _, line := range strings.Split(string(out), "\n") {
		if strings.HasPrefix(line, "idle_cpu_percent=") {
			e.Metrics["idle_cpu_percent"] = strings.TrimPrefix(line, "idle_cpu_percent=")
		}
	}
	if err != nil {
		os.Stderr.Write(out)
	}
	return e
}
