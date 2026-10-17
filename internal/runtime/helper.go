package runtime

import (
	"context"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strconv"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func superviseHelper(ctx context.Context, cfg Config, secretFile string) error {
	if runtime.GOOS != "darwin" {
		return contracts.ErrUnsupported
	}
	home := filepath.Join(cfg.Home, "helper")
	if err := os.MkdirAll(home, 0700); err != nil {
		return err
	}
	args := []string{"-n", "-W", "-g", "-a", cfg.HelperApp, "--args", "session", "--socket", cfg.Socket, "--secret-file", secretFile, "--home", home, "--gateway-pid", strconv.Itoa(os.Getpid())}
	if cfg.HelperLabCommands {
		args = append(args, "--lab-commands")
	}
	for ctx.Err() == nil {
		cmd := exec.CommandContext(ctx, "open", args...)
		cmd.Stdout = os.Stdout
		cmd.Stderr = os.Stderr
		err := cmd.Run()
		if ctx.Err() != nil {
			return nil
		}
		if err != nil {
			return err
		}
		timer := time.NewTimer(250 * time.Millisecond)
		select {
		case <-ctx.Done():
			timer.Stop()
			return nil
		case <-timer.C:
		}
	}
	return nil
}
