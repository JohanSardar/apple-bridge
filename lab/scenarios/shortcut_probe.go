package scenarios

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/shortcut"
)

type shortcutProbeCLI interface {
	Inventory(context.Context, bool, string) ([]shortcut.Entry, error)
	Run(context.Context, shortcut.Input) (shortcut.Result, error)
}

func runShortcutProbe(ctx context.Context, s Scenario, level, name string, cli shortcutProbeCLI, qualified bool, accepts func([]byte) bool) Evidence {
	e := Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, At: time.Now().UTC(), Passed: true, Metrics: map[string]any{}}
	add := func(name string, passed bool) {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: passed})
		e.Passed = e.Passed && passed
	}
	limited := func(reason string) Evidence {
		e.Status, e.Passed = "limited", false
		e.Conditions = map[string]string{"reason": reason}
		return e
	}
	entries, err := cli.Inventory(ctx, false, "")
	add("inventory_available", err == nil)
	if err != nil {
		return limited("Shortcuts inventory could not be read. Open Shortcuts and check that your library is available, then retry.")
	}
	matches := 0
	for _, entry := range entries {
		if entry.Name == name {
			matches++
		}
	}
	e.Metrics["installed_shortcut_count"] = len(entries)
	e.Metrics["probe_pack_matches"] = matches
	add("unique_probe_pack_installed", matches == 1)
	if matches != 1 {
		return limited("Install exactly one dedicated " + name + " shortcut using packaging/shortcuts/screen-time-probe.md or design/spec/shortcuts.md, then retry. No shortcut was run.")
	}
	add("probe_pack_qualified", qualified)
	if !qualified {
		return limited("Export and qualify the dedicated " + name + " shortcut before running this probe. Inventory alone does not establish its actions or permissions. No shortcut was run.")
	}
	timeout := 20 * time.Second
	if level == "real" {
		timeout = 5 * time.Minute
	}
	result, err := cli.Run(ctx, shortcut.Input{Shortcut: name, Timeout: timeout})
	digest := sha256.Sum256(result.Data)
	e.Metrics["output_bytes"] = len(result.Data)
	e.Metrics["output_sha256"] = hex.EncodeToString(digest[:])
	e.Metrics["timed_out"] = result.TimedOut
	add("bounded_runner_completed", err == nil && result.ExitStatus == 0 && !result.TimedOut)
	add("probe_output_valid", len(result.Data) > 0 && len(result.Data) <= 32<<10 && accepts(result.Data))
	if !e.Passed {
		return limited("The dedicated " + name + " shortcut did not return valid output. Open it in Shortcuts, check its actions and existing permissions, and retry after it runs successfully.")
	}
	return e
}

type shortcutProbeSimulation struct {
	entries []shortcut.Entry
	result  shortcut.Result
	err     error
	runs    int
}

func (f *shortcutProbeSimulation) Inventory(context.Context, bool, string) ([]shortcut.Entry, error) {
	return f.entries, f.err
}
func (f *shortcutProbeSimulation) Run(context.Context, shortcut.Input) (shortcut.Result, error) {
	f.runs++
	return f.result, f.err
}
