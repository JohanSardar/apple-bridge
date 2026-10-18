package scenarios

import (
	"context"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"time"

	"github.com/JohanSardar/apple-bridge/lab/world"
)

func cleanupNativeMessageRun(w *world.World, runID string, groupDraft bool) (bool, map[string]any) {
	ctx, cancel := context.WithTimeout(context.Background(), 150*time.Second)
	defer cancel()
	report := filepath.Join(os.Getenv("HOME"), "Library", "Application Support", "AppleBridgeDev", "messages-cleanup-"+runID+".json")
	defer os.Remove(report)
	tag := "[ab-test " + runID + "]"
	args := []string{"-n", "-W", "-a", w.HelperApp, "--args", "probe", "cleanup-tagged-self", "--tag", tag, "--approved-contacts", "--report", report}
	if groupDraft {
		args = append(args, "--group-draft")
	}
	err := exec.CommandContext(ctx, "open", args...).Run()
	metrics := map[string]any{}
	data, readErr := os.ReadFile(report)
	json.Unmarshal(data, &metrics)
	truth, truthErr := w.RecoverTruth("observe", map[string]any{"source": "messages", "tag": tag})
	rows, _ := truth["records"].([]any)
	remaining := 0
	activeKnown := true
	for _, raw := range rows {
		row, _ := raw.(map[string]any)
		active, exists := row["active"].(bool)
		activeKnown = activeKnown && exists
		if active {
			remaining++
		}
	}
	metrics["appletruth_active_remaining"] = remaining
	metrics["appletruth_verified"] = truthErr == nil && truth["status"] == "verified" && activeKnown
	return err == nil && readErr == nil && metrics["passed"] == true && metrics["appletruth_verified"] == true && remaining == 0, metrics
}
