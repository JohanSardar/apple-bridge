package runtime

import (
	"context"
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"os"
	"os/exec"
	"path/filepath"
	"time"
)

func Permissions(ctx context.Context, app, name string) (map[string]string, error) {
	command := "permission-status"
	if name == "fake" {
		command = "permission-fake"
	} else if name != "" {
		command = "permission-request"
	}
	return permissionReport(ctx, app, command, name)
}

func PermissionStatusProof(ctx context.Context, app string) (map[string]string, error) {
	return permissionReport(ctx, app, "permission-status-proof", "")
}

func permissionReport(ctx context.Context, app, command, name string) (map[string]string, error) {
	if app == "" {
		app = filepath.Join(os.Getenv("HOME"), "Applications", "AppleBridgeDev", "AppleBridgeHelper.app")
	}
	report := filepath.Join(os.Getenv("HOME"), "Library", "Application Support", "AppleBridgeDev", "permissions-"+contracts.NewID("run")+".json")
	defer os.Remove(report)
	bounded, cancel := context.WithTimeout(ctx, 3*time.Minute)
	defer cancel()
	if err := exec.CommandContext(bounded, "open", "-n", "-W", "-g", "-a", app, "--args", command, name, "--report", report).Run(); err != nil {
		return nil, err
	}
	data, err := os.ReadFile(report)
	if err != nil {
		return nil, err
	}
	var states map[string]string
	err = json.Unmarshal(data, &states)
	return states, err
}
