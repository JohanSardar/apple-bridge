package scenarios

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"

	"github.com/JohanSardar/apple-bridge/internal/shortcut"
)

type clockTimerNativeRunner struct{ app string }

func (r clockTimerNativeRunner) Run(ctx context.Context, input shortcut.Input) (shortcut.Result, error) {
	result := shortcut.Result{ExitStatus: -1}
	directory, err := os.MkdirTemp("", "ab-clock-timer-")
	if err != nil {
		return result, err
	}
	defer os.RemoveAll(directory)
	request, report := filepath.Join(directory, "request"), filepath.Join(directory, "report")
	diagnostic := filepath.Join(directory, "diagnostic")
	if err = os.WriteFile(request, input.Data, 0600); err != nil {
		return result, err
	}
	command := exec.CommandContext(ctx, "open", "-n", "-W", "-g", "-a", r.app, "--stderr", diagnostic, "--args", "clock-timers", request, report)
	if err = command.Run(); err != nil {
		fmt.Fprintln(os.Stderr, "clock_timer_helper_launch_completed=false")
		return result, err
	}
	result.Data, err = os.ReadFile(report)
	var requestValue struct {
		Operation string `json:"operation"`
	}
	if json.Unmarshal(input.Data, &requestValue) == nil && requestValue.Operation == "recent_timers" {
		var response struct {
			Version   int    `json:"version"`
			Operation string `json:"operation"`
			Timers    []struct {
				ID       string `json:"id"`
				Duration int    `json:"duration"`
				Running  bool   `json:"running"`
			} `json:"timers"`
		}
		decoded := json.Unmarshal(result.Data, &response) == nil
		invalid, unique := 0, map[string]bool{}
		for _, timer := range response.Timers {
			if timer.ID == "" || unique[timer.ID] || timer.Duration < 1 || timer.Duration > 86400 {
				invalid++
			}
			unique[timer.ID] = true
		}
		diagnostic, _ := json.Marshal(map[string]any{"recent_report_written": err == nil, "recent_output_bytes": len(result.Data), "recent_schema_valid": decoded, "recent_version_valid": response.Version == 1, "recent_operation_matches": response.Operation == requestValue.Operation, "recent_timers_present": response.Timers != nil, "recent_rows": len(response.Timers), "recent_invalid_rows": invalid})
		fmt.Fprintln(os.Stderr, string(diagnostic))
	}
	if err == nil {
		result.ExitStatus = 0
	} else if data, readErr := os.ReadFile(diagnostic); readErr == nil {
		os.Stderr.Write(data)
	}
	return result, err
}
