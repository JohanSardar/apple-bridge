package runtime

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	goruntime "runtime"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func MessagesSelfTest(ctx context.Context, app string, force, includeUI bool) (json.RawMessage, error) {
	return messagesSelfTest(ctx, app, force, includeUI, "messages")
}

func MessagesParitySelfTest(ctx context.Context, app string, force, includeUI bool) (json.RawMessage, error) {
	return messagesSelfTest(ctx, app, force, includeUI, "messages-parity")
}

func messagesSelfTest(ctx context.Context, app string, force, includeUI bool, variant string) (json.RawMessage, error) {
	if goruntime.GOOS != "darwin" {
		return nil, contracts.ErrUnsupported
	}
	if app == "" {
		app = filepath.Join(os.Getenv("HOME"), "Applications", "AppleBridgeDev", "AppleBridgeHelper.app")
	}
	report := filepath.Join(os.Getenv("HOME"), "Library", "Application Support", "AppleBridgeDev", "messages-selftest-cli-"+contracts.NewID("run")+".json")
	defer os.Remove(report)
	bounded, cancel := context.WithTimeout(ctx, 10*time.Minute)
	defer cancel()
	args := []string{"-n", "-W", "-g", "-a", app, "--args", "selftest", variant, "--report", report}
	if !includeUI {
		args = append(args, "--no-ui")
	}
	if force {
		args = append(args, "--force")
	}
	if err := exec.CommandContext(bounded, "open", args...).Run(); err != nil {
		return nil, err
	}
	data, err := os.ReadFile(report)
	if err != nil {
		return nil, err
	}
	var result struct {
		State string `json:"state"`
	}
	if json.Unmarshal(data, &result) != nil || result.State == "" {
		return nil, contracts.ErrCorrupt
	}
	if result.State != "proven" {
		return data, fmt.Errorf("messages self-test %s; failed operations remain unproven", result.State)
	}
	return data, nil
}
