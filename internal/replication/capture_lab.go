//go:build lab

package replication

import (
	"context"
	"os"
	"path/filepath"
	"time"
)

func captureFeatures() []string {
	if _, err := os.Stat(filepath.Join(os.Getenv("APPLE_BRIDGE_HOME"), "lab-capture-legacy")); err == nil {
		return nil
	}
	return []string{"capture_atomic"}
}

func capturePartBoundary(ctx context.Context) error {
	home := os.Getenv("APPLE_BRIDGE_HOME")
	if os.Remove(filepath.Join(home, "lab-capture-part-armed")) != nil {
		return nil
	}
	if err := os.WriteFile(filepath.Join(home, "lab-capture-part-reached"), []byte("partial_staged"), 0600); err != nil {
		return err
	}
	for {
		if _, err := os.Stat(filepath.Join(home, "lab-capture-part-release")); err == nil {
			return nil
		}
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-time.After(10 * time.Millisecond):
		}
	}
}
