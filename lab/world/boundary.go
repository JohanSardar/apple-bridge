package world

import (
	"os"
	"path/filepath"
	"time"
)

func (w *World) ArmBoundary(endpoint Endpoint, boundary string) error {
	if endpoint.Remote {
		return os.ErrInvalid
	}
	if err := os.Remove(filepath.Join(endpoint.Home, "lab-"+boundary+"-reached")); err != nil && !os.IsNotExist(err) {
		return err
	}
	return os.WriteFile(filepath.Join(endpoint.Home, "lab-"+boundary+"-armed"), nil, 0600)
}

func (w *World) AwaitBoundary(endpoint Endpoint, boundary, expected string) error {
	return w.Await(10*time.Second, func() bool {
		data, err := os.ReadFile(filepath.Join(endpoint.Home, "lab-"+boundary+"-reached"))
		return err == nil && string(data) == expected
	})
}
