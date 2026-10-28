package scenarios

import (
	"context"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"
)

func runRecordings(ctx context.Context, s Scenario, level string) Evidence {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e := Evidence{GitSHA: strings.TrimSpace(string(sha)), RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, At: time.Now().UTC(), Hosts: []string{"air"}}
	if s.ID == "VMEMO-LATE-ASSETS" {
		e.Assertions = []Assertion{{Name: "late_assets_not_implemented", Passed: false}}
		return e
	}
	binary, err := recordingsChecks(ctx)
	e.Assertions = []Assertion{{Name: "recordings_native_fixture_build", Passed: err == nil}}
	if err == nil {
		err = exec.CommandContext(ctx, binary).Run()
		e.Assertions = append(e.Assertions, Assertion{Name: "metadata_wal_insert_update_delete", Passed: err == nil})
	}

	e.Passed = err == nil
	return e
}

func recordingsChecks(ctx context.Context) (string, error) {
	output, err := os.ReadFile("native/.build/native-output-1")
	binary := filepath.Join(strings.TrimSpace(string(output)), "RecordingsChecks")
	info, statErr := os.Stat(binary)
	fresh := err == nil && statErr == nil
	if fresh {
		for _, root := range []string{"native/Package.swift", "packaging/dev/build-native.sh", "packaging/dev/build-native.py", "native/Sources/SourceSupport", "native/Sources/VoiceMemosSource", "native/Sources/VoicemailSource", "native/Sources/RecordingsChecks"} {
			walkErr := filepath.Walk(root, func(_ string, entry os.FileInfo, err error) error {
				if err != nil {
					return err
				}
				if !entry.IsDir() && entry.ModTime().After(info.ModTime()) {
					fresh = false
				}
				return nil
			})
			if walkErr != nil {
				fresh = false
			}
		}
	}
	if fresh {
		return binary, nil
	}
	cmd := exec.CommandContext(ctx, "packaging/dev/build-native.sh")
	cmd.Env = append(os.Environ(), "APPLE_BRIDGE_NATIVE_TARGET=RecordingsChecks")
	if err := cmd.Run(); err != nil {
		return "", err
	}
	output, err = os.ReadFile("native/.build/native-output-1")
	return filepath.Join(strings.TrimSpace(string(output)), "RecordingsChecks"), err
}
