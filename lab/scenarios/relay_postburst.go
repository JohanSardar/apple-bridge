package scenarios

import (
	"context"
	"errors"
	"fmt"
	"net"
	"os"
	"path/filepath"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func relayPostBurstChecks(ctx context.Context, w *world.World, e *Evidence, add func(string, bool)) {
	tag := "[ab-test " + contracts.NewID("relay-postburst") + "]"
	_, err := w.Truth("fixture", map[string]any{"action": "create_list", "list": "Bridge Test", "tag": tag})
	add("post_burst_fixture_list", err == nil)
	if err != nil {
		return
	}
	defer func() {
		_, err := w.RecoverTruth("fixture", map[string]any{"action": "cleanup", "list": "Bridge Test", "tag": tag})
		add("post_burst_tagged_fixture_cleanup", err == nil)
	}()
	input := relayProbeInput(w)
	input["Title"] = tag + " post-burst delete"
	input["DeleteID"] = contracts.NewCommandID()
	var result struct {
		InitialHTTP int     `json:"initial_http_status"`
		HTTP        int     `json:"http_status"`
		Code        string  `json:"error_code"`
		State       string  `json:"state"`
		InitialMS   float64 `json:"initial_response_ms"`
		ElapsedMS   float64 `json:"elapsed_ms"`
		TTLMS       int64   `json:"normal_ttl_ms"`
		Observed    bool    `json:"observed"`
		LocalAbsent bool    `json:"local_absent"`
		CommandID   string  `json:"command_id"`
	}
	err = w.RelayExchange("postburst", input, func() (any, error) {
		truth, err := w.Truth("observe", map[string]any{"list": "Bridge Test", "tag": tag, "title": input["Title"]})
		records, _ := truth["records"].([]any)
		add("post_burst_fixture_independently_visible", err == nil && len(records) == 1)
		if err != nil || len(records) != 1 {
			return nil, fmt.Errorf("post-burst fixture truth failed")
		}
		relayNativeChecks(ctx, w, e, add)
		return map[string]bool{"burst_finished": true}, nil
	}, &result)
	add("post_burst_forwarded_delete_result_measured", err == nil && result.TTLMS > 0)
	if err != nil {
		return
	}
	e.Metrics["post_burst_delete_command_id"] = result.CommandID
	e.Metrics["post_burst_delete_initial_http_status"] = result.InitialHTTP
	e.Metrics["post_burst_delete_http_status"] = result.HTTP
	e.Metrics["post_burst_delete_error_code"] = result.Code
	e.Metrics["post_burst_delete_state"] = result.State
	e.Metrics["post_burst_delete_initial_response_ms"] = result.InitialMS
	e.Metrics["post_burst_delete_elapsed_ms"] = result.ElapsedMS
	e.Metrics["post_burst_delete_normal_ttl_ms"] = result.TTLMS
	succeeded := result.Observed && result.LocalAbsent && result.ElapsedMS <= float64(result.TTLMS)
	assertion := Assertion{Name: "post_burst_forwarded_delete_within_normal_TTL", Passed: succeeded}
	if !succeeded && result.HTTP == 503 && result.Code == "unavailable" {
		assertion.Status = "blocked"
		assertion.Detail = fmt.Sprintf("LIVE-FIX: reminders.delete HTTP 503 unavailable after %.3f ms; normal TTL %d ms; no redispatch (ADR 0014)", result.ElapsedMS, result.TTLMS)
		e.Status = "blocked"
	}
	e.Assertions = append(e.Assertions, assertion)
	if succeeded {
		truth, err := w.Truth("observe", map[string]any{"list": "Bridge Test", "tag": tag, "title": input["Title"]})
		records, _ := truth["records"].([]any)
		add("post_burst_forwarded_delete_independent_native_absence", err == nil && len(records) == 0)
	}
	relayPostBurstBlobCheck(w, e)
}

func relayPostBurstBlobCheck(w *world.World, e *Evidence) {
	started := time.Now()
	err := w.BlobControl(map[string]string{"action": "relay-fixture"}, nil)
	elapsed := float64(time.Since(started)) / float64(time.Millisecond)
	status, code := 200, ""
	if err != nil {
		status, code = 0, "other_error"
		var timeout net.Error
		if errors.As(err, &timeout) && timeout.Timeout() {
			code = "timeout"
		}
		if count, _ := fmt.Sscanf(err.Error(), "blob lab status %d", &status); count == 1 {
			code = fmt.Sprintf("http_%d", status)
		}
		os.WriteFile(filepath.Join(w.Dir, "post-burst-blob-error.log"), []byte(err.Error()), 0600)
	}
	e.Metrics["post_burst_blob_stage_elapsed_ms"] = elapsed
	e.Metrics["post_burst_blob_stage_http_status"] = status
	e.Metrics["post_burst_blob_stage_error_code"] = code
	assertion := Assertion{Name: "post_burst_blob_stage_available", Passed: err == nil}
	if code == "timeout" || status == 500 {
		assertion.Status = "blocked"
		assertion.Detail = fmt.Sprintf("LIVE-FIX: native blob fixture %s after %.3f ms; default control timeout unchanged", code, elapsed)
		e.Status = "blocked"
	}
	e.Assertions = append(e.Assertions, assertion)
}
