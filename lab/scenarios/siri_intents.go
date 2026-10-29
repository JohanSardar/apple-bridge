package scenarios

import (
	"context"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"syscall"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func runSiriIntents(ctx context.Context, s Scenario, level string) Evidence {
	e := Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, At: time.Now().UTC()}
	dir, err := os.MkdirTemp("", "ab-siri-intents-")
	if err != nil {
		return e
	}
	defer os.RemoveAll(dir)
	app := filepath.Join(dir, "AppleBridgeHelper.app")
	command := exec.CommandContext(ctx, "packaging/dev/build-helper.sh", app)
	command.SysProcAttr = &syscall.SysProcAttr{Setpgid: true}
	command.Cancel = func() error { return syscall.Kill(-command.Process.Pid, syscall.SIGKILL) }
	err = command.Run()
	e.Assertions = append(e.Assertions, Assertion{Name: "helper_compiles_and_bundles", Passed: err == nil})
	if err != nil {
		return e
	}
	metadata, err := os.ReadFile(filepath.Join(app, "Contents/Resources/Metadata.appintents/extract.actionsdata"))
	emitted := err == nil && strings.Contains(string(metadata), "BridgeStatusIntent")
	e.Assertions = append(e.Assertions, Assertion{Name: "status_intent_metadata_emitted", Passed: emitted})
	e.Status = "limited"
	reason := "Johan must verify Apple Bridge status appears in Shortcuts and runs."
	if !emitted {
		reason = "App Intents metadata missing from Helper bundle; full Xcode metadata toolchain required. " + reason
	}
	e.Conditions = map[string]string{"reason": reason}
	return e
}
