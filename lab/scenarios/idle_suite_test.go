package scenarios

import (
	"context"
	"errors"
	"testing"
	"time"
)

func TestExpandIdleSuite(t *testing.T) {
	t.Chdir("../..")
	grouped, err := expandIdleSuite([]Scenario{{ID: "Q2-IDLE"}})
	if err != nil || len(grouped) != 11 {
		t.Fatalf("scenarios=%d error=%v", len(grouped), err)
	}
	seen := map[string]bool{}
	for _, scenario := range grouped {
		if !idleScenario(scenario.ID) || seen[scenario.ID] {
			t.Fatalf("invalid shared idle scenario %s", scenario.ID)
		}
		seen[scenario.ID] = true
	}
	standalone, err := expandIdleSuite([]Scenario{{ID: "MAC-IDLE"}})
	if err != nil || len(standalone) != 1 || standalone[0].ID != "MAC-IDLE" {
		t.Fatal("changed standalone scenario selection")
	}
}

func TestSharedIdleWindow(t *testing.T) {
	started := time.Date(2026, 10, 30, 0, 0, 0, 0, time.UTC)
	clock := started
	samples, waits := 0, 0
	window := measureIdleWindow(context.Background(), true, func() (float64, error) { samples++; return float64(samples) * 3, nil }, func() time.Time { return clock }, func(_ context.Context, d time.Duration) error {
		waits++
		if d != 5*time.Minute {
			t.Fatalf("duration %v", d)
		}
		clock = clock.Add(d)
		return nil
	})
	if samples != 2 || waits != 1 || !window.passed || window.percent != 1 {
		t.Fatalf("samples=%d waits=%d window=%+v", samples, waits, window)
	}
	for _, id := range []string{"VIS-IDLE", "SC-IDLE", "MAC-IDLE", "SAF-IDLE", "LOC-IDLE", "MAIL-IDLE", "FOC-IDLE", "CALL-IDLE", "MAP-IDLE", "MUS-IDLE", "PHO-IDLE"} {
		if !idleScenario(id) {
			t.Fatalf("missing %s", id)
		}
		e := idleWindowEvidence(Evidence{Scenario: id, Passed: true, Metrics: map[string]any{}}, window, true, true, 0, Evidence{})
		if !e.Passed || e.Metrics["shared_window"] != started.Format(time.RFC3339Nano) || e.Metrics["idle_seconds"] != float64(300) || e.Metrics["helper_cpu_percent"] != float64(1) {
			t.Fatalf("%s: %+v", id, e)
		}
	}
	if idleScenario("FILE-IDLE") || idleScenario("MAC-CAPTURE") {
		t.Fatal("unrelated scenario grouped")
	}
	failed := idleWindowEvidence(Evidence{Passed: false, Metrics: map[string]any{}}, window, true, true, 0, Evidence{})
	if failed.Passed {
		t.Fatal("lost setup failure")
	}
}

func TestIdleWindowFailures(t *testing.T) {
	for _, tc := range []struct {
		name                         string
		settled                      bool
		before, after                float64
		sampleErr, afterErr, waitErr error
		elapsed                      time.Duration
	}{
		{name: "not settled"},
		{name: "budget", settled: true, after: 6, elapsed: 5 * time.Minute},
		{name: "regression", settled: true, before: 10, after: 1, elapsed: 5 * time.Minute},
		{name: "sample failure", settled: true, sampleErr: errors.New("ps")},
		{name: "final sample failure", settled: true, afterErr: errors.New("ps"), elapsed: 5 * time.Minute},
		{name: "cancelled", settled: true, waitErr: context.Canceled},
		{name: "short", settled: true, elapsed: time.Minute},
	} {
		t.Run(tc.name, func(t *testing.T) {
			clock := time.Now()
			samples, waits := 0, 0
			w := measureIdleWindow(context.Background(), tc.settled, func() (float64, error) {
				samples++
				if samples == 1 {
					return tc.before, tc.sampleErr
				}
				return tc.after, tc.afterErr
			}, func() time.Time { return clock }, func(context.Context, time.Duration) error { waits++; clock = clock.Add(tc.elapsed); return tc.waitErr })
			if w.passed {
				t.Fatalf("passed: %+v", w)
			}
			if !tc.settled && (samples != 0 || waits != 0) {
				t.Fatal("measured before settling")
			}
		})
	}
}

func TestIdleWindowLimitedEvidence(t *testing.T) {
	e := idleWindowEvidence(Evidence{Passed: true, Metrics: map[string]any{}}, idleWindow{}, false, false, 0, Evidence{Status: "limited", Conditions: map[string]string{"reason": "missing grant"}})
	if e.Passed || e.Status != "limited" || e.Conditions["reason"] != "missing grant" {
		t.Fatalf("%+v", e)
	}
	if _, exists := e.Metrics["helper_cpu_percent"]; exists {
		t.Fatal("invented CPU sample")
	}
	if _, exists := e.Metrics["shared_window"]; exists {
		t.Fatal("invented measurement window")
	}
}
