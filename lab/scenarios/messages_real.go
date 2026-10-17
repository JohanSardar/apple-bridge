package scenarios

import (
	"bytes"
	"context"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"time"

	"github.com/JohanSardar/apple-bridge/lab/world"
)

func runRealMessages(ctx context.Context, s Scenario, level string, evidence Evidence) Evidence {
	replicas := 0
	if level == "replica" || s.ID == "MSG-INCOMING" || s.ID == "MSG-RECEIPTS" {
		replicas = 1
	}
	if level == "replica" {
		evidence.Hosts = append(evidence.Hosts, "ubuntu")
	}
	w, err := world.NewReal(ctx, level, replicas, world.BlobOptions{Scope: "messages:read,messages:write,system:read"})
	if err != nil {
		evidence.Assertions = append(evidence.Assertions, Assertion{Name: "real_world_start", Passed: false})
		return evidence
	}
	defer w.Close()
	evidence.NetworkPath = w.NetworkPath
	request := filepath.Join(w.Dir, "messages-check.json")
	data, err := json.Marshal(map[string]any{"scenario": s.ID, "gateway": w.Gateway, "replicas": w.Replicas, "history_started": filepath.Join(w.Dir, "history-started")})
	if err == nil {
		err = os.WriteFile(request, data, 0600)
	}
	if err != nil {
		evidence.Assertions = append(evidence.Assertions, Assertion{Name: "private_truth_request", Passed: false})
		return evidence
	}
	defer os.Remove(request)
	command := exec.CommandContext(ctx, "python3", "lab/bench/messages-real.py", request)
	var output []byte
	if s.ID == "MSG-HISTORY-IMPORT" {
		var buffer bytes.Buffer
		command.Stdout = &buffer
		err = command.Start()
		if err == nil {
			marker := filepath.Join(w.Dir, "history-started")
			ready := w.Await(45*time.Second, func() bool { _, err := os.Stat(marker); return err == nil }) == nil
			if ready {
				send := Evidence{RunID: evidence.RunID, Latency: map[string]Latency{}}
				send = runNativeMessagesCommandsInWorld(ctx, Scenario{ID: "MSG-SEND-SELF-TEXT"}, level, send, w)
				evidence.Assertions = append(evidence.Assertions, send.Assertions...)
			} else {
				evidence.Assertions = append(evidence.Assertions, Assertion{Name: "history_mutation_ready", Passed: false})
			}
			err = command.Wait()
			output = buffer.Bytes()
		}
	} else {
		output, err = command.Output()
	}
	var result struct {
		Assertions []Assertion          `json:"assertions"`
		Samples    map[string][]float64 `json:"samples"`
		Status     string               `json:"status"`
		Metrics    map[string]any       `json:"metrics"`
	}
	if err != nil || json.Unmarshal(output, &result) != nil {
		evidence.Assertions = append(evidence.Assertions, Assertion{Name: "redacted_readonly_truth_check", Passed: false})
		return evidence
	}
	evidence.Assertions = append(evidence.Assertions, result.Assertions...)
	evidence.Status = result.Status
	evidence.Metrics = result.Metrics
	for name, values := range result.Samples {
		evidence.Latency[name] = stats(values, 0)
	}
	evidence.Passed = len(evidence.Assertions) > 0 && evidence.Status == ""
	for _, assertion := range evidence.Assertions {
		evidence.Passed = evidence.Passed && assertion.Passed
	}
	return evidence
}
