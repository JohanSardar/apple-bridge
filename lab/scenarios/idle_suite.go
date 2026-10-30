package scenarios

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/runtime"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func idleScenario(id string) bool {
	switch id {
	case "VIS-IDLE", "SC-IDLE", "MAC-IDLE", "SAF-IDLE", "LOC-IDLE", "MAIL-IDLE", "FOC-IDLE", "CALL-IDLE", "MAP-IDLE", "MUS-IDLE", "PHO-IDLE":
		return true
	}
	return false
}

func idleSuite(ctx context.Context, scenarios []Scenario, asJSON bool) error {
	ctx, cancel := context.WithTimeout(ctx, 40*time.Minute)
	defer cancel()
	w, err := world.NewReal(ctx, "real", 0, world.BlobOptions{Scope: "admin"})
	if err != nil {
		return err
	}
	defer w.Close()
	client := scripted.Client{Endpoint: w.Gateway}
	ready := w.Await(30*time.Second, func() bool {
		var out contracts.QueryResult
		return client.Call(ctx, "helper.version", contracts.Query{Args: json.RawMessage(`{}`)}, &out) == nil
	}) == nil
	results := make([]Evidence, len(scenarios))
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	for i, s := range scenarios {
		e := Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: "real", At: time.Now().UTC(), GitSHA: strings.TrimSpace(string(sha)), Passed: true, Metrics: map[string]any{}, Hosts: []string{"air"}}
		add := func(name string, pass bool) {
			e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: pass})
			e.Passed = e.Passed && pass
		}
		add("helper_ready", ready)
		collection := ""
		switch s.ID {
		case "SC-IDLE":
			collection = "shortcuts.shortcut"
		case "CALL-IDLE":
			collection = "phone.call"
		case "MAIL-IDLE":
			collection = "mail.message"
		case "VIS-IDLE":
			add("doctor_document_sources_ready", w.Await(15*time.Second, func() bool {
				r, err := runtime.Doctor(ctx, w.Gateway.Home)
				return err == nil && r.DocumentChecks["vision"] == "ready" && r.DocumentChecks["pdf"] == "ready"
			}) == nil)
		case "FOC-IDLE":
			report := filepath.Join(os.Getenv("HOME"), "Library", "Application Support", "AppleBridgeDev", e.RunID+"-focus.json")
			problem := exec.CommandContext(ctx, "open", "-n", "-W", "-g", "-a", w.HelperApp, "--args", "focus-real-checks", "FOC-STATUS-REAL", "--report", report).Run()
			raw, readErr := os.ReadFile(report)
			os.Remove(report)
			var probe map[string]bool
			add("focus_status_readback", problem == nil && readErr == nil && json.Unmarshal(raw, &probe) == nil && probe["grant_passed"] && probe["status_passed"])
		case "PHO-IDLE":
			report := filepath.Join(os.Getenv("HOME"), "Library", "Application Support", "AppleBridgeDev", e.RunID+"-photos.json")
			problem := exec.CommandContext(ctx, "open", "-n", "-W", "-g", "-a", w.HelperApp, "--args", "photos-real-checks", s.ID, "--report", report).Run()
			raw, readErr := os.ReadFile(report)
			os.Remove(report)
			var probe map[string]any
			add("helper_report", problem == nil && readErr == nil && json.Unmarshal(raw, &probe) == nil)
			for _, name := range []string{"grant_passed", "read_passed", "import_passed", "original_passed", "cleanup_passed"} {
				add(name, probe[name] == true)
			}
			for _, name := range []string{"asset_count", "original_digest"} {
				e.Metrics[name] = probe[name]
			}
		}
		if collection != "" {
			add("capture_ready", w.Await(60*time.Second, func() bool {
				var rows contracts.ReadResult
				return client.Call(ctx, "records.query", contracts.ReadQuery{Collection: collection, Limit: 1}, &rows) == nil && len(rows.Records) > 0
			}) == nil)
		}
		results[i] = e
	}
	settled := awaitIdleCapture(ctx, w, &results[0])
	pidRaw, pidErr := os.ReadFile(filepath.Join(w.Gateway.Home, "helper", "helper.pid"))
	pid := strings.TrimSpace(string(pidRaw))
	cpu := func() (float64, error) {
		raw, err := exec.CommandContext(ctx, "ps", "-p", pid, "-o", "time=").Output()
		if err != nil {
			return 0, err
		}
		fields := strings.Split(strings.TrimSpace(string(raw)), ":")
		if len(fields) < 2 {
			return 0, fmt.Errorf("invalid CPU time")
		}
		total := 0.0
		for _, field := range fields {
			n, err := strconv.ParseFloat(field, 64)
			if err != nil {
				return 0, err
			}
			total = total*60 + n
		}
		return total, nil
	}
	quiet := 0
	warmup := time.Now()
	previous, previousErr := cpu()
	for quiet < 2 && time.Since(warmup) < 5*time.Minute {
		start := time.Now()
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-time.After(10 * time.Second):
		}
		next, nextErr := cpu()
		if previousErr == nil && nextErr == nil && next >= previous && 100*(next-previous)/time.Since(start).Seconds() < 2 {
			quiet++
		} else {
			quiet = 0
		}
		previous, previousErr = next, nextErr
	}
	before, beforeErr := cpu()
	started := time.Now()
	select {
	case <-ctx.Done():
		return ctx.Err()
	case <-time.After(5 * time.Minute):
	}
	after, afterErr := cpu()
	seconds := time.Since(started).Seconds()
	percent := 100 * (after - before) / seconds
	var version contracts.QueryResult
	responsive := client.Call(ctx, "helper.version", contracts.Query{Args: json.RawMessage(`{}`), DeadlineMS: 1000}, &version) == nil
	passed := quiet == 2 && responsive && settled && pidErr == nil && beforeErr == nil && afterErr == nil && after >= before && seconds >= 300 && percent < 2
	failed := false
	for _, e := range results {
		e.Assertions = append(e.Assertions, Assertion{Name: "messages_capture_and_media_settled", Passed: settled}, Assertion{Name: "helper_reached_idle", Passed: quiet == 2}, Assertion{Name: "shared_five_minute_helper_cpu_under_two_percent", Passed: passed})
		e.Passed = e.Passed && passed
		e.Metrics["helper_cpu_percent"] = percent
		e.Metrics["idle_seconds"] = seconds
		e.Metrics["shared_window"] = started.UTC().Format(time.RFC3339Nano)
		if err := Save(e); err != nil {
			return err
		}
		if asJSON {
			json.NewEncoder(os.Stdout).Encode(e)
		} else {
			fmt.Printf("%s real passed=%t\n", e.Scenario, e.Passed)
		}
		failed = failed || !e.Passed
	}
	if failed {
		return fmt.Errorf("idle scenario suite failed")
	}
	return nil
}
