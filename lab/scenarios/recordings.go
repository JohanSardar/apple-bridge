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
	e := Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, At: time.Now().UTC(), Hosts: []string{"air"}}
	if s.ID == "VMEMO-LATE-ASSETS" {
		e.Assertions = []Assertion{{Name: "late_assets_not_implemented", Passed: false}}
		return e
	}
	cmd := exec.CommandContext(ctx, "packaging/dev/build-native.sh")
	cmd.Env = append(os.Environ(), "APPLE_BRIDGE_NATIVE_TARGET=RecordingsChecks")
	err := cmd.Run()
	e.Assertions = []Assertion{{Name: "recordings_native_fixture_build", Passed: err == nil}}
	if err == nil {
		directory, readErr := os.ReadFile("native/.build/native-output-1")
		if readErr == nil {
			err = exec.CommandContext(ctx, filepath.Join(strings.TrimSpace(string(directory)), "RecordingsChecks")).Run()
		} else {
			err = readErr
		}
		name := "metadata_wal_insert_update_delete"
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: err == nil})
	}
	e.Passed = err == nil
	return e
}
