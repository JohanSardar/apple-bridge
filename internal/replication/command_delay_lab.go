//go:build lab

package replication

import (
	"context"
	"os"
	"path/filepath"
	"strconv"
	"time"
)

func commandApplyDelay(ctx context.Context) error {
	path := filepath.Join(os.Getenv("APPLE_BRIDGE_HOME"), "command-apply-delay")
	b, err := os.ReadFile(path)
	if err != nil {
		return nil
	}
	if os.Remove(path) != nil {
		return nil
	}
	ms, err := strconv.ParseInt(string(b), 10, 64)
	if err != nil || ms < 0 || ms > 5000 {
		return nil
	}
	timer := time.NewTimer(time.Duration(ms) * time.Millisecond)
	defer timer.Stop()
	select {
	case <-ctx.Done():
		return ctx.Err()
	case <-timer.C:
		return nil
	}
}
