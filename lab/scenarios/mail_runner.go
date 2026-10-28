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

func runMail(ctx context.Context, s Scenario, level string) Evidence {
	e := Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, At: time.Now().UTC(), Passed: true}
	build := exec.CommandContext(ctx, "packaging/dev/build-native.sh")
	build.Env = append(os.Environ(), "APPLE_BRIDGE_NATIVE_TARGET=MailChecks")
	output, err := build.CombinedOutput()
	e.Assertions = append(e.Assertions, Assertion{Name: "native_mail_build", Passed: err == nil})
	if err != nil {
		os.Stderr.Write(output)
		e.Passed = false
		return e
	}
	directory, err := os.ReadFile("native/.build/native-output-1")
	if err == nil {
		output, err = exec.CommandContext(ctx, filepath.Join(strings.TrimSpace(string(directory)), "MailChecks"), s.ID).CombinedOutput()
	}
	e.Assertions = append(e.Assertions, Assertion{Name: "native_mail_fixture", Passed: err == nil})
	e.Passed = err == nil
	if err != nil {
		os.Stderr.Write(output)
	}
	return e
}
