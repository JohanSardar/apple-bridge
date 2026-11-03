package scenarios

import (
	"context"
	"fmt"
	"os"
	"runtime"
	"sort"
	"strings"
	"sync"
	"time"

	"github.com/JohanSardar/apple-bridge/lab/world"
)

type quietKey struct{}

func RequireQuiet(ctx context.Context) context.Context {
	return context.WithValue(ctx, quietKey{}, true)
}

type HostConditions struct {
	Start        map[string]world.HostSample `json:"-"`
	Quiet        bool                        `json:"quiet"`
	RequireQuiet bool                        `json:"-"`
	WaitMS       int64                       `json:"-"`
	WaitExpired  bool                        `json:"-"`
	Summary      string                      `json:"summary"`
}

func hostSamples(ctx context.Context, level string) map[string]world.HostSample {
	result := map[string]world.HostSample{}
	var local, remote world.HostSample
	var wg sync.WaitGroup
	wg.Add(1)
	go func() { defer wg.Done(); local = world.SampleHost(ctx, false) }()
	if level == "replica" && !world.ImplementationSelection(ctx).LocalReplicas {
		wg.Add(1)
		go func() { defer wg.Done(); remote = world.SampleHost(ctx, true) }()
	}
	wg.Wait()
	localName := "air"
	if runtime.GOOS == "linux" {
		localName = "ubuntu"
	}
	result[localName] = local
	if level == "replica" && !world.ImplementationSelection(ctx).LocalReplicas {
		result["ubuntu"] = remote
	}
	return result
}
func samplesQuiet(samples map[string]world.HostSample) bool {
	for _, sample := range samples {
		if !sample.Quiet {
			return false
		}
	}
	return len(samples) > 0
}

func Run(ctx context.Context, s Scenario, level string) (e Evidence) {
	selection := world.ImplementationSelection(ctx)
	if (selection.Gateway == "rust" || selection.Replica == "rust") && !(level == "sim" && selection.Gateway == "rust" && selection.Replica == "go" && strings.HasPrefix(s.ID, "MSG-")) && !(level == "sim" && selection.Gateway == "rust" && selection.Replica == "go" && (calendarGatewaySimScenario(s) || nativeGatewaySimScenario(s))) && !(level == "sim" && selection.Gateway == "go" && nativeReplicaSimScenario(s)) && !(level == "real" && selection.LocalReplicas && selection.Gateway == "go" && nativeReplicaRealScenario(s)) {
		if reason := replicaQualificationBlocker(s); reason != "" {
			return Evidence{Scenario: s.ID, Level: level, At: time.Now().UTC(), Status: "blocked", Impl: "mixed", Implementations: selection, HostConditions: &HostConditions{}, Assertions: []Assertion{{Name: "binary_harness_required", Detail: reason}}}
		}
	}
	ctx, captureMetrics := world.WithCaptureMetrics(ctx)
	conditions := HostConditions{}
	required, _ := ctx.Value(quietKey{}).(bool)
	conditions.RequireQuiet = required && latencyCritical(s)
	waitStart := time.Now()
	conditions.Start = hostSamples(ctx, level)
	if conditions.RequireQuiet {
		deadline := waitStart.Add(60 * time.Second)
		for !samplesQuiet(conditions.Start) && time.Now().Before(deadline) && ctx.Err() == nil {
			timer := time.NewTimer(3 * time.Second)
			select {
			case <-ctx.Done():
				timer.Stop()
			case <-timer.C:
			}
			conditions.Start = hostSamples(ctx, level)
		}
		conditions.WaitMS = time.Since(waitStart).Milliseconds()
		conditions.WaitExpired = !samplesQuiet(conditions.Start)
		if conditions.WaitExpired {
			fmt.Fprintf(os.Stderr, "quiet wait expired for %s; continuing with non-quiet evidence\n", s.ID)
		}
	}
	conditions.Quiet = samplesQuiet(conditions.Start)
	if s.Feature == "packaging.release" {
		conditions.Start["pro"] = conditions.Start["air"]
		delete(conditions.Start, "air")
	}
	hosts := make([]string, 0, len(conditions.Start))
	for host := range conditions.Start {
		hosts = append(hosts, host)
	}
	sort.Strings(hosts)
	summaries := make([]string, 0, len(hosts))
	for _, host := range hosts {
		sample := conditions.Start[host]
		idle := "unknown"
		if sample.CPUIdle != nil {
			idle = fmt.Sprintf("%.0f%%", *sample.CPUIdle)
		}
		summaries = append(summaries, fmt.Sprintf("%s: load=%v cpu_idle=%s cpus=%d other_heavy=%d", host, sample.Load, idle, sample.CPUCount, sample.OtherHeavySlots))
	}
	conditions.Summary = strings.Join(summaries, "; ")
	reports := diagnosticReportsDir()
	before, reportsErr := helperCrashReports(reports)
	e = runScenario(ctx, s, level)
	e.Implementations = world.ImplementationSelection(ctx)
	if e.Impl == "" {
		e.Impl = e.Implementations.Replica
		if e.Implementations.Gateway != e.Implementations.Replica {
			e.Impl = "mixed"
		}
	}
	if reportsErr == nil {
		if strings.HasPrefix(s.ID, "NOTE-") {
			e = failOnNotesHelperCrashes(e, reports, before)
		} else {
			e = failOnHelperCrashes(e, reports, before)
		}
	}
	e.HostConditions = &conditions
	e.Hosts = hosts
	if e.Metrics == nil {
		e.Metrics = map[string]any{}
	}
	resets := captureMetrics.Resets()
	switch previous := e.Metrics["capture_observer_resets"].(type) {
	case int:
		resets += previous
	case float64:
		resets += int(previous)
	}
	e.Metrics["capture_observer_resets"] = resets
	if s.ID == "CORE-REPLICA-CAPTURE-OBSERVERS" || s.ID == "WEB-UPLOAD-REPLICA-STATUS-CAPTURE" {
		e.Assertions = append(e.Assertions, Assertion{Name: "capture_observer_resets_counted", Passed: resets > 0})
		e.Passed = e.Passed && resets > 0
	} else if s.Feature == "core.replication" || s.ID == "WEB-LIVE-READMODELS" || strings.HasPrefix(s.ID, "WEB-RELAY-") {
		e.Assertions = append(e.Assertions, Assertion{Name: "no_oversized_capture_observer_resets", Passed: resets == 0})
		e.Passed = e.Passed && resets == 0
	}
	return e
}

func latencyCritical(s Scenario) bool {
	if s.ID == "CORE-BENCH" || s.ID == "CORE-LIVE-LATENCY" {
		return true
	}
	for _, step := range s.Then {
		if _, ok := step["latency"]; ok {
			return true
		}
	}
	return false
}
