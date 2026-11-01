package scenarios

import (
	"context"
	"crypto/sha256"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"syscall"

	"github.com/JohanSardar/apple-bridge/lab/world"
)

func notesNativeSetup(ctx context.Context) world.NativeSetup {
	return world.NativeSetup{IndependentTruth: true, BeforeStart: func(w *world.World) error {
		packagePath, err := filepath.Abs("lab/truth/native")
		if err != nil {
			return err
		}
		if resolved, err := filepath.EvalSymlinks(packagePath); err == nil {
			packagePath = resolved
		}
		lock := filepath.Join(os.TempDir(), fmt.Sprintf("ab-notes-truth-%x.lock", sha256.Sum256([]byte(packagePath))))
		command := exec.CommandContext(ctx, "packaging/dev/build-truth.sh", w.TruthApp)
		command.Env = append(os.Environ(), "APPLE_BRIDGE_NATIVE_LOCK="+lock)
		command.SysProcAttr = &syscall.SysProcAttr{Setpgid: true}
		command.Cancel = func() error { return syscall.Kill(-command.Process.Pid, syscall.SIGKILL) }
		if output, err := command.CombinedOutput(); err != nil {
			fmt.Fprintln(os.Stderr, string(output))
			return fmt.Errorf("Notes verification app could not be built: %w", err)
		}
		return nil
	}}
}
