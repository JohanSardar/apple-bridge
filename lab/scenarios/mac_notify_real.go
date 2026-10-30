package scenarios

import (
	"context"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"sort"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func macNotifyReal(ctx context.Context, w *world.World, add func(string, bool)) {
	report := filepath.Join(os.Getenv("HOME"), "Library", "Application Support", "AppleBridgeDev", contracts.NewID("notify-proof")+".json")
	defer os.Remove(report)
	ctx, cancel := context.WithTimeout(ctx, 30*time.Second)
	defer cancel()
	err := exec.CommandContext(ctx, "open", "-n", "-W", "-g", "-a", w.HelperApp, "--args", "mac-notify-proof", "--report", report).Run()
	add("signed_helper_notification_probe", err == nil)
	if err != nil {
		return
	}
	data, err := os.ReadFile(report)
	var proof map[string]bool
	if err == nil {
		err = json.Unmarshal(data, &proof)
	}
	add("notification_report", err == nil && len(proof) > 0)
	names := make([]string, 0, len(proof))
	for name := range proof {
		names = append(names, name)
	}
	sort.Strings(names)
	for _, name := range names {
		add(name, proof[name])
	}
}
