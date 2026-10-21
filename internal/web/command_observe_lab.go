//go:build lab

package web

import (
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"os"
	"path/filepath"
)

func commandObserveFailure(stage string) error {
	path := filepath.Join(os.Getenv("APPLE_BRIDGE_HOME"), "command-observe-failure")
	raw, err := os.ReadFile(path)
	if err == nil && string(raw) == stage && os.Remove(path) == nil {
		return contracts.ErrReset
	}
	return nil
}
