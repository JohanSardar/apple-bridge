//go:build lab

package operations

import (
	"os"
	"path/filepath"
	"strconv"
	"time"
)

func gatewayNow() time.Time {
	b, _ := os.ReadFile(filepath.Join(os.Getenv("APPLE_BRIDGE_HOME"), "clock-offset"))
	offset, _ := strconv.ParseInt(string(b), 10, 64)
	return time.Now().UTC().Add(time.Duration(offset) * time.Millisecond)
}
