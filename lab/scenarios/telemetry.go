package scenarios

import (
	"context"
	"fmt"
	"os"
	"sync"
	"time"

	"github.com/JohanSardar/apple-bridge/lab/world"
)

type quietKey struct{}

func RequireQuiet(ctx context.Context) context.Context {
	return context.WithValue(ctx, quietKey{}, true)
}

type HostConditions struct {
	QuietDefinition string                        `json:"quiet_definition"`
	Start           map[string]world.HostSample   `json:"start"`
	End             map[string]world.HostSample   `json:"end"`
	Samples         []map[string]world.HostSample `json:"periodic_samples,omitempty"`
	Quiet           bool                          `json:"quiet"`
	RequireQuiet    bool                          `json:"require_quiet"`
	WaitMS          int64                         `json:"quiet_wait_ms"`
	WaitExpired     bool                          `json:"quiet_wait_expired"`
}

func hostSamples(ctx context.Context, level string) map[string]world.HostSample {
	result := map[string]world.HostSample{}
	var local, remote world.HostSample
	var wg sync.WaitGroup
	wg.Add(1)
	go func() { defer wg.Done(); local = world.SampleHost(ctx, false) }()
	if level == "replica" {
		wg.Add(1)
		go func() { defer wg.Done(); remote = world.SampleHost(ctx, true) }()
	}
	wg.Wait()
	result["air"] = local
	if level == "replica" {
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
	conditions := HostConditions{QuietDefinition: "Load averages are ordered 1/5/15 minutes. Every observed host sample: no other heavy.sh slot active and one-minute load below logical CPU count; CPU idle is a one-second sample; free memory excludes cache. Sampled at start, every five seconds, and end."}
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
	monitor, cancel := context.WithCancel(ctx)
	done := make(chan struct{})
	go func() {
		defer close(done)
		ticker := time.NewTicker(5 * time.Second)
		defer ticker.Stop()
		for {
			select {
			case <-monitor.Done():
				return
			case <-ticker.C:
				sample := hostSamples(monitor, level)
				if monitor.Err() != nil {
					return
				}
				conditions.Samples = append(conditions.Samples, sample)
			}
		}
	}()
	e = runScenario(ctx, s, level)
	cancel()
	<-done
	endCtx, endCancel := context.WithTimeout(context.Background(), 10*time.Second)
	conditions.End = hostSamples(endCtx, level)
	endCancel()
	conditions.Quiet = conditions.Quiet && samplesQuiet(conditions.End)
	for _, sample := range conditions.Samples {
		conditions.Quiet = conditions.Quiet && samplesQuiet(sample)
	}
	e.HostConditions = &conditions
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
