package scenarios

import (
	"context"
	"os"
	"os/exec"
	"path/filepath"

	"github.com/JohanSardar/apple-bridge/internal/shortcut"
)

type clockTimerNativeRunner struct{ app string }

func (r clockTimerNativeRunner) Run(ctx context.Context, input shortcut.Input) (shortcut.Result, error) {
	result := shortcut.Result{ExitStatus: -1}
	directory, err := os.MkdirTemp("", "ab-clock-timer-")
	if err != nil {
		return result, err
	}
	defer os.RemoveAll(directory)
	request, report := filepath.Join(directory, "request"), filepath.Join(directory, "report")
	if err = os.WriteFile(request, input.Data, 0600); err != nil {
		return result, err
	}
	command := exec.CommandContext(ctx, "open", "-n", "-W", "-g", "-a", r.app, "--args", "clock-timers", request, report)
	if err = command.Run(); err != nil {
		return result, err
	}
	result.Data, err = os.ReadFile(report)
	if err == nil {
		result.ExitStatus = 0
	}
	return result, err
}
