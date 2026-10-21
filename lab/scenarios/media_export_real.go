package scenarios

import (
	"bytes"
	"context"
	"database/sql"
	"encoding/json"
	"os"
	"path/filepath"
	"regexp"
	"strconv"
	"strings"
	"syscall"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func exportTemporaryFiles(digests []string) []string {
	var files []string
	pattern := regexp.MustCompile(`^[0-9a-fA-F-]{36}\.(mp4|m4a|mov)$`)
	for _, digest := range digests {
		directory := filepath.Join(os.Getenv("HOME"), "Library", "Application Support", "AppleBridge", "media-derivatives", "v1", digest)
		entries, _ := os.ReadDir(directory)
		for _, entry := range entries {
			if pattern.MatchString(entry.Name()) {
				files = append(files, filepath.Join(directory, entry.Name()))
			}
		}
	}
	return files
}

func runRealMediaExportSafety(ctx context.Context, e Evidence) Evidence {
	e.Metrics = map[string]any{"synthetic_only": false, "writes_submitted": 0, "media_drain_enabled": os.Getenv("APPLE_BRIDGE_LAB_PAUSE_MEDIA") != "1"}
	add := func(name string, passed bool) {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: passed})
	}
	input, err := os.ReadFile(os.Getenv("APPLE_BRIDGE_EXPORT_PENDING_DIGESTS"))
	var digests []string
	valid := err == nil && json.Unmarshal(input, &digests) == nil && len(digests) > 0
	for _, digest := range digests {
		valid = valid && regexp.MustCompile(`^[0-9a-f]{64}$`).MatchString(digest)
	}
	if !valid {
		e.Status = "environment-limited"
		e.Assertions = []Assertion{{Name: "private_pending_lookup_valid", Status: "unverified", Detail: "Provide a private metadata-only pending-digest lookup using APPLE_BRIDGE_EXPORT_PENDING_DIGESTS"}}
		return e
	}
	add("private_pending_lookup_valid", true)
	if os.Getenv("APPLE_BRIDGE_LAB_PAUSE_MEDIA") == "1" {
		add("media_drain_enabled", false)
		return e
	}
	priorPath := os.Getenv("APPLE_BRIDGE_EXPORT_PRIOR_REAL_PROOF")
	var prior struct {
		RunID                string `json:"run_id"`
		Passed               bool   `json:"passed"`
		Yield                bool   `json:"yield_observed"`
		Export               bool   `json:"export_attempt_on_backlog"`
		GuardInputsUnchanged bool   `json:"guard_inputs_unchanged"`
		Compute              bool   `json:"compute_observed"`
	}
	if priorPath != "" {
		data, problem := os.ReadFile(priorPath)
		add("prior_capture_proof_valid", problem == nil && json.Unmarshal(data, &prior) == nil && prior.Passed && prior.Compute && prior.Yield && prior.Export && prior.GuardInputsUnchanged)
		if len(e.Assertions) > 1 && !e.Assertions[len(e.Assertions)-1].Passed {
			return e
		}
		e.Metrics["prior_real_capture"] = prior.RunID
	}
	initialTemporary := exportTemporaryFiles(digests)
	initialNames := map[string]bool{}
	for _, name := range initialTemporary {
		initialNames[name] = true
	}
	reports := diagnosticReportsDir()
	before, reportErr := helperCrashReports(reports)
	add("diagnostic_reports_readable", reportErr == nil)
	if reportErr != nil {
		return e
	}
	w, err := world.NewReal(ctx, "real", 0, world.BlobOptions{Scope: "messages:read,system:read"})
	add("signed_real_world_ready", err == nil)
	if err != nil {
		return e
	}
	closed := false
	defer func() {
		if !closed {
			w.Close()
		}
	}()
	pidPath := filepath.Join(w.Gateway.Home, "helper", "helper.pid")
	var pid []byte
	pidErr := w.Await(30*time.Second, func() bool {
		var problem error
		pid, problem = os.ReadFile(pidPath)
		return problem == nil && len(bytes.TrimSpace(pid)) > 0
	})
	_, pauseErr := os.Stat(filepath.Join(w.Gateway.Home, "helper", "lab-pause-media"))
	add("media_drain_not_paused", os.IsNotExist(pauseErr))
	ready := w.Await(2*time.Minute, func() bool {
		var result contracts.ReadResult
		_, problem := world.Request(ctx, w.Gateway, "/v1/op/records.query", contracts.ReadQuery{Collection: "system.source", Limit: 100}, &result)
		for _, record := range result.Records {
			var data struct {
				Source string `json:"source"`
				State  string `json:"state"`
			}
			if json.Unmarshal(record.Data, &data) == nil && data.Source == "messages" && data.State == "ready" {
				return problem == nil
			}
		}
		return false
	})
	add("messages_source_ready", ready == nil)
	type manifest struct {
		AwaitingLive bool                       `json:"awaitingLive"`
		Derivatives  map[string]json.RawMessage `json:"derivatives"`
		Unavailable  map[string]string          `json:"unavailable"`
	}
	states := []map[string]any{}
	attempted := false
	terminal := w.Await(3*time.Minute, func() bool {
		for _, name := range exportTemporaryFiles(digests) {
			attempted = attempted || !initialNames[name]
		}
		current := []map[string]any{}
		for _, digest := range digests {
			data, problem := os.ReadFile(filepath.Join(os.Getenv("HOME"), "Library", "Application Support", "AppleBridge", "media-derivatives", "v1", digest, "manifest.json"))
			var value manifest
			if problem != nil || json.Unmarshal(data, &value) != nil || value.AwaitingLive {
				return false
			}
			present := len(value.Derivatives["video"]) > 0 || len(value.Derivatives["live"]) > 0 || len(value.Derivatives["audio"]) > 0
			if !present && len(value.Unavailable) == 0 {
				return false
			}
			current = append(current, map[string]any{"present": present, "unavailable": value.Unavailable})
		}
		states = current
		return true
	})
	add("previously_pending_audiovisual_terminal", terminal == nil)
	database, dbErr := sql.Open("sqlite", "file:"+filepath.Join(w.Gateway.Home, "state.db")+"?mode=ro")
	captured := dbErr == nil
	if dbErr == nil {
		captured = w.Await(time.Minute, func() bool {
			for _, digest := range digests {
				var found bool
				problem := database.QueryRowContext(ctx, "SELECT EXISTS(SELECT 1 FROM records WHERE collection='messages.attachment' AND deleted=0 AND json_extract(payload,'$.data.blob.digest')=?)", digest).Scan(&found)
				if problem != nil || !found {
					return false
				}
			}
			return true
		}) == nil
		database.Close()
	}
	add("pending_records_captured_without_loss", captured)
	stable := pidErr == nil
	until := time.Now().Add(15 * time.Second)
	for stable && time.Now().Before(until) {
		if sleep(ctx, 250*time.Millisecond) != nil {
			stable = false
			break
		}
		after, problem := os.ReadFile(pidPath)
		stable = problem == nil && bytes.Equal(pid, after)
	}
	add("same_helper_survives_media_drain", stable)
	private := filepath.Join("lab", "evidence", "private", "export-safety", e.RunID)
	data, _ := json.MarshalIndent(states, "", "  ")
	os.WriteFile(private+"-real-outcomes.json", data, 0o600)
	trace, traceErr := os.ReadFile(filepath.Join(w.Gateway.Home, "helper", "media-work-trace.jsonl"))
	os.WriteFile(private+"-media-work-trace.jsonl", trace, 0o600)
	compute := traceErr == nil && bytes.Contains(trace, []byte(`"media_compute_started"`)) && bytes.Contains(trace, []byte(`"media_compute_finished"`))
	yielded := bytes.Contains(trace, []byte(`"media_yielded_to_foreground"`))
	if priorPath != "" {
		compute = compute || prior.Compute
		yielded = yielded || prior.Yield
		attempted = attempted || prior.Export
	}
	add("real_media_compute_observed", compute)
	add("actual_foreground_yield_observed", yielded)
	add("export_path_attempt_on_prior_backlog", attempted)
	guardLog, guardErr := os.Stat(filepath.Join(w.Gateway.Home, "helper", "dispatch-guard.log"))
	add("no_command_dispatched", guardErr == nil && guardLog.Size() == 0)
	w.Close()
	closed = true
	helperPID, _ := strconv.Atoi(strings.TrimSpace(string(pid)))
	add("helper_clean_shutdown", helperPID > 1 && w.Await(5*time.Second, func() bool { return syscall.Kill(helperPID, 0) == syscall.ESRCH }) == nil)
	add("export_temporaries_removed_after_restart_and_shutdown", len(exportTemporaryFiles(digests)) == 0)
	add("helper_crash_free", len(newHelperCrashes(reports, before)) == 0)
	e.Metrics["pending_items_terminal"] = terminal == nil
	e.Metrics["initial_abandoned_exports_present"] = len(initialTemporary) > 0
	e.Passed = true
	for _, assertion := range e.Assertions {
		e.Passed = e.Passed && assertion.Passed
	}
	return e
}
