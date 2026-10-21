//go:build lab

package web

import (
	"os"
	"path/filepath"
	"strconv"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func commandObserveFailure(stage string) error {
	path := filepath.Join(os.Getenv("APPLE_BRIDGE_HOME"), "command-observe-failure")
	raw, err := os.ReadFile(path)
	if err == nil && string(raw) == stage && os.Remove(path) == nil {
		return contracts.ErrReset
	}
	return nil
}

func commandStatusLookup(id string) {
	home := os.Getenv("APPLE_BRIDGE_HOME")
	raw, err := os.ReadFile(filepath.Join(home, "command-status-probe"))
	if err != nil || string(raw) != id {
		return
	}
	file, err := os.OpenFile(filepath.Join(home, "command-status-lookups"), os.O_CREATE|os.O_APPEND|os.O_WRONLY, 0600)
	if err != nil {
		return
	}
	defer file.Close()
	file.WriteString(strconv.FormatInt(time.Now().UnixNano(), 10) + "\n")
}
func commandVisibilityHeld(id string) bool {
	home := os.Getenv("APPLE_BRIDGE_HOME")
	raw, err := os.ReadFile(filepath.Join(home, "command-visibility-hold"))
	if err != nil || string(raw) != id {
		return false
	}
	os.WriteFile(filepath.Join(home, "command-status-held"), []byte(id), 0600)
	return true
}
