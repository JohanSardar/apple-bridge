package scenarios

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func safariCleanupItems(ctx context.Context, w *world.World, run, action string, item cleanupItem) ([]cleanupItem, error) {
	ctx, cancel := context.WithTimeout(ctx, 20*time.Second)
	defer cancel()
	id := contracts.NewID("safari-cleanup")
	input := filepath.Join(w.Dir, id+"-request.json")
	output := filepath.Join(w.Dir, id+"-report.json")
	defer os.Remove(input)
	defer os.Remove(output)
	data, err := json.Marshal(map[string]any{"run_id": run, "action": action, "item": item})
	if err != nil {
		return nil, err
	}
	if err = os.WriteFile(input, data, 0600); err != nil {
		return nil, err
	}
	if err = exec.CommandContext(ctx, "open", "-n", "-W", "-g", "-a", w.HelperApp, "--args", "safari-run-cleanup", "--request", input, "--report", output).Run(); err != nil {
		return nil, err
	}
	data, err = os.ReadFile(output)
	if err != nil {
		return nil, err
	}
	var report struct {
		Items   []cleanupItem `json:"items"`
		Passed  bool          `json:"passed"`
		Limited bool          `json:"limited"`
	}
	if json.Unmarshal(data, &report) != nil || report.Limited || action == "snapshot" && report.Items == nil || action != "snapshot" && !report.Passed {
		return nil, fmt.Errorf("Safari owned cleanup unavailable")
	}
	return report.Items, nil
}
