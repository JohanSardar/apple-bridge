package scenarios

import (
	"context"
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"os"
	"os/exec"
	"path/filepath"
	"time"
)

func awaitIdleCapture(ctx context.Context, w *world.World, e *Evidence) bool {
	home, err := os.UserHomeDir()
	if err != nil {
		return false
	}
	report := filepath.Join(home, "Library", "Application Support", "AppleBridgeDev", e.RunID+"-idle-permissions.json")
	defer os.Remove(report)
	statusCtx, cancel := context.WithTimeout(ctx, 15*time.Second)
	defer cancel()
	err = exec.CommandContext(statusCtx, "open", "-n", "-W", "-g", "-a", w.HelperApp, "--args", "status", "--report", report).Run()
	data, readErr := os.ReadFile(report)
	var permissions map[string]string
	if err != nil || readErr != nil || json.Unmarshal(data, &permissions) != nil {
		return false
	}
	if permissions["full_disk_access"] != "granted" {
		e.Status = "limited"
		e.Conditions = map[string]string{"reason": "needs Full Disk Access for the current signed AppleBridgeHelper on the Air; non-prompting status=" + permissions["full_disk_access"] + "; capture cannot settle, so no CPU measurement was taken"}
		e.Assertions = append(e.Assertions, Assertion{Name: "full_disk_access_granted", Passed: false})
		return false
	}

	return w.Await(25*time.Minute, func() bool {
		data, err := os.ReadFile(filepath.Join(w.Gateway.Home, "helper", "messages-idle.json"))
		var state struct {
			Idle bool `json:"idle"`
		}
		return err == nil && json.Unmarshal(data, &state) == nil && state.Idle
	}) == nil
}
