package scenarios

import (
	"context"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
)

func iphoneProjectCommand(ctx context.Context, name string, args ...string) *exec.Cmd {
	cmd := exec.CommandContext(ctx, name, args...)
	root := os.Getenv("APPLE_BRIDGE_IPHONE_ROOT")
	if root == "" {
		cmd.Err = fmt.Errorf("set APPLE_BRIDGE_IPHONE_ROOT to a checkout of JohanSardar/apple-bridge-iphone for this integration scenario")
		return cmd
	}
	root, err := filepath.Abs(root)
	if err == nil {
		_, err = os.Stat(filepath.Join(root, "ios", "project.yml"))
	}
	if err != nil {
		cmd.Err = fmt.Errorf("iPhone checkout: %w", err)
		return cmd
	}
	cmd.Dir = root
	return cmd
}
