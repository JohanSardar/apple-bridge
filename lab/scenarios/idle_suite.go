package scenarios

import (
	"context"
	"crypto/sha256"
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

func expandIdleSuite(scenarios []Scenario) ([]Scenario, error) {
	if len(scenarios) != 1 || scenarios[0].ID != "Q2-IDLE" {
		return scenarios, nil
	}
	return Load([]string{
		"lab/scenarios/vision/VIS-IDLE.yaml", "lab/scenarios/mac/MAC-IDLE.yaml",
		"lab/scenarios/mail/MAIL-IDLE.yaml", "lab/scenarios/phone/CALL-IDLE.yaml",
		"lab/scenarios/safari/SAF-IDLE.yaml", "lab/scenarios/maps/LOC-IDLE.yaml",
		"lab/scenarios/maps/MAP-IDLE.yaml", "lab/scenarios/music/MUS-IDLE.yaml",
		"lab/scenarios/photos/PHO-IDLE.yaml", "lab/scenarios/shortcuts/FOC-IDLE.yaml",
		"lab/scenarios/shortcuts/SC-IDLE.yaml",
	})
}

func saveIdleGroup(level string, asJSON bool, result error) error {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e := Evidence{RunID: contracts.NewID("run"), Scenario: "Q2-IDLE", Level: level, GitSHA: strings.TrimSpace(string(sha)), At: time.Now().UTC(), Passed: result == nil, Assertions: []Assertion{{Name: "all_eleven_idle_scenarios_pass", Passed: result == nil}}}
	if err := Save(e); err != nil {
		return fmt.Errorf("save shared idle result: %w", err)
	}
	if asJSON {
		json.NewEncoder(os.Stdout).Encode(e)
	} else {
		fmt.Printf("Q2-IDLE %s passed=%t\n", level, e.Passed)
	}
	return result
}

func idleSuite(ctx context.Context, scenarios []Scenario, asJSON bool) error {
	ctx, cancel := context.WithTimeout(ctx, 40*time.Minute)
	defer cancel()
	ctx, captureMetrics := world.WithCaptureMetrics(ctx)
	samples := hostSamples(ctx, "real")
	sample := samples["air"]
	idle := "unknown"
	if sample.CPUIdle != nil {
		idle = fmt.Sprintf("%.0f%%", *sample.CPUIdle)
	}
	conditions := HostConditions{Start: samples, Quiet: samplesQuiet(samples), Summary: fmt.Sprintf("air: load=%v cpu_idle=%s cpus=%d other_heavy=%d", sample.Load, idle, sample.CPUCount, sample.OtherHeavySlots)}
	reports := diagnosticReportsDir()
	crashReports, reportsErr := helperCrashReports(reports)
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
		case "MAC-IDLE", "MUS-IDLE":
			collection = "mac.power"
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
	settleStarted := time.Now()
	settled := awaitIdleCapture(ctx, w, &results[0])
	settleSeconds := time.Since(settleStarted).Seconds()
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
	window := measureIdleWindow(ctx, settled && pidErr == nil, cpu, time.Now, func(ctx context.Context, duration time.Duration) error {
		timer := time.NewTimer(duration)
		defer timer.Stop()
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-timer.C:
			return nil
		}
	})
	profileDigest := ""
	if window.sampled {
		directory := filepath.Join("lab/evidence/private", "idle")
		if os.MkdirAll(directory, 0700) == nil {
			profile := filepath.Join(directory, results[0].RunID+".sample")
			if os.WriteFile(profile, nil, 0600) == nil {
				bounded, stop := context.WithTimeout(ctx, 10*time.Second)
				problem := exec.CommandContext(bounded, "sample", pid, "5", "10", "-file", profile).Run()
				stop()
				if bytes, readErr := os.ReadFile(profile); problem == nil && readErr == nil {
					profileDigest = fmt.Sprintf("%x", sha256.Sum256(bytes))
				}
			}
		}
	}
	var version contracts.QueryResult
	responsive := client.Call(ctx, "helper.version", contracts.Query{Args: json.RawMessage(`{}`), DeadlineMS: 1000}, &version) == nil
	currentPID, currentPIDErr := os.ReadFile(filepath.Join(w.Gateway.Home, "helper", "helper.pid"))
	passed := responsive && currentPIDErr == nil && strings.TrimSpace(string(currentPID)) == pid && window.passed
	failed := false
	for _, e := range results {
		e = idleWindowEvidence(e, window, settled, passed, settleSeconds, results[0])
		e.Metrics["helper_pid"] = pid
		if profileDigest != "" {
			e.Metrics["helper_sample_sha256"] = profileDigest
		}
		e.Metrics["capture_observer_resets"] = captureMetrics.Resets()
		e.HostConditions = &conditions
		if reportsErr == nil {
			e = failOnHelperCrashes(e, reports, crashReports)
		}
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

type idleWindow struct {
	started time.Time
	seconds float64
	percent float64
	sampled bool
	passed  bool
}

func measureIdleWindow(ctx context.Context, settled bool, cpu func() (float64, error), now func() time.Time, wait func(context.Context, time.Duration) error) idleWindow {
	if !settled {
		return idleWindow{}
	}
	before, beforeErr := cpu()
	if beforeErr != nil {
		return idleWindow{}
	}
	window := idleWindow{started: now()}
	if wait(ctx, 5*time.Minute) != nil {
		return window
	}
	after, afterErr := cpu()
	window.seconds = now().Sub(window.started).Seconds()
	window.sampled = afterErr == nil && after >= before && window.seconds >= 300
	if window.sampled {
		window.percent = 100 * (after - before) / window.seconds
		window.passed = window.percent < 2
	}
	return window
}

func idleWindowEvidence(e Evidence, window idleWindow, settled, passed bool, settleSeconds float64, diagnostics Evidence) Evidence {
	e.Assertions = append(e.Assertions, Assertion{Name: "messages_capture_and_media_settled", Passed: settled}, Assertion{Name: "shared_five_minute_helper_cpu_under_two_percent", Passed: passed})
	e.Passed = e.Passed && passed
	if window.sampled {
		e.Metrics["helper_cpu_percent"] = window.percent
	}
	e.Metrics["idle_seconds"] = window.seconds
	e.Metrics["settle_seconds"] = settleSeconds
	if !settled {
		e.Status = diagnostics.Status
		e.Conditions = diagnostics.Conditions
	}
	if !window.started.IsZero() {
		e.Metrics["shared_window"] = window.started.UTC().Format(time.RFC3339Nano)
	}
	return e
}
