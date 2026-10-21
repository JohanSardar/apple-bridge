package scenarios

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"os"
	"os/exec"
	"path/filepath"
	"time"
)

func runAgentCaptureRejection(ctx context.Context, w *world.World, add func(string, bool)) {
	locker := exec.CommandContext(ctx, "sqlite3", filepath.Join(w.Gateway.Home, "state.db"))
	input, inputErr := locker.StdinPipe()
	output, outputErr := locker.StdoutPipe()
	if inputErr != nil || outputErr != nil || locker.Start() != nil {
		add("capture_rejection_lock_process", false)
		return
	}
	defer func() { input.Close(); locker.Process.Kill(); locker.Wait() }()
	fmt.Fprintln(input, "BEGIN IMMEDIATE; SELECT 1;")
	ready := bufio.NewScanner(output)
	if !ready.Scan() || ready.Text() != "1" {
		add("capture_rejection_write_lock", false)
		return
	}
	add("capture_rejection_fixture", w.ControlAction("/messages", map[string]any{"action": "incoming", "key": contracts.NewID("busy")}) == nil)
	status := func() (string, string, int) {
		var result contracts.ReadResult
		code, err := world.Request(ctx, w.Gateway, "/v1/op/sources.status", map[string]any{}, &result)
		if err != nil || code != 200 {
			return "", "", 0
		}
		for _, record := range result.Records {
			var data struct {
				Source string `json:"source"`
				State  string `json:"state"`
				Reason string `json:"reason"`
				Count  int    `json:"capture_rejected_count"`
				Last   string `json:"last_capture_rejected_reason"`
			}
			if json.Unmarshal(record.Data, &data) == nil && data.Source == "messages" {
				if data.State == "degraded" {
					return data.State, data.Reason, data.Count
				}
				return data.State, data.Last, data.Count
			}
		}
		return "", "", 0
	}
	err := w.Await(8*time.Second, func() bool {
		state, reason, count := status()
		return state == "degraded" && reason == "gateway_capture_rejected:sqlite_5" && count > 0
	})
	add("capture_rejection_degraded_and_counted", err == nil)
	log, logErr := os.ReadFile(filepath.Join(w.Dir, "gateway.log"))
	add("capture_rejection_logged", logErr == nil && bytes.Contains(log, []byte(`"stage":"gateway_capture_rejected"`)) && bytes.Contains(log, []byte(`"error_code":"sqlite_5"`)))
	input.Close()
	locker.Wait()
	err = w.Await(5*time.Second, func() bool {
		state, reason, count := status()
		return state == "ready" && reason == "gateway_capture_rejected:sqlite_5" && count > 0
	})
	add("capture_rejection_count_retained_after_reconnect", err == nil)
}
