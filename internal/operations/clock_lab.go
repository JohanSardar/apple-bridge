//go:build lab

package operations

import (
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"os"
	"path/filepath"
	"strconv"
	"time"
)

func gatewayNow() time.Time {
	if os.Getenv("APPLE_BRIDGE_LAB_REPLAY") != "" {
		return contracts.DecisionNow()
	}
	b, _ := os.ReadFile(filepath.Join(os.Getenv("APPLE_BRIDGE_HOME"), "clock-offset"))
	offset, _ := strconv.ParseInt(string(b), 10, 64)
	return time.Now().UTC().Add(time.Duration(offset) * time.Millisecond)
}

func dropCommandResponse() bool {
	return os.Remove(filepath.Join(os.Getenv("APPLE_BRIDGE_HOME"), "drop-command-response")) == nil
}
