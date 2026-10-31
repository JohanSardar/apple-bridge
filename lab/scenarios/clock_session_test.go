package scenarios

import (
	"context"
	"reflect"
	"slices"
	"testing"

	"github.com/JohanSardar/apple-bridge/internal/clockpack"
)

func TestClockSessionRestoresAlarmAndTimerBaseline(t *testing.T) {
	for _, id := range []string{"CLK-ALARM-REAL", "CLK-TIMER-REAL"} {
		fixture := newClockSimulation()
		alarms, timers := slices.Clone(fixture.alarms), slices.Clone(fixture.timers)
		e := runClockSession(context.Background(), Scenario{ID: id}, Evidence{RunID: "synthetic-run", Passed: true, Metrics: map[string]any{}}, clockpack.Pack{Runner: fixture})
		if !e.Passed || !reflect.DeepEqual(alarms, fixture.alarms) || !reflect.DeepEqual(timers, fixture.timers) {
			t.Fatal(id, e, fixture)
		}
	}
}

func TestClockTimerRequiresCleanupBeforeStarting(t *testing.T) {
	fixture := newClockSimulation()
	fixture.capabilities = []string{"start_timer"}
	e := runClockTimer(context.Background(), Evidence{RunID: "synthetic-run", Passed: true, Metrics: map[string]any{}}, clockpack.Pack{Runner: fixture})
	if e.Passed || slices.Contains(fixture.operations, "start_timer") {
		t.Fatal(e, fixture.operations)
	}
}

func TestClockTimerCleansUpAfterLostCreationReply(t *testing.T) {
	fixture := newClockSimulation()
	before := slices.Clone(fixture.timers)
	fixture.failStart = true
	e := runClockTimer(context.Background(), Evidence{RunID: "synthetic-run", Passed: true, Metrics: map[string]any{}}, clockpack.Pack{Runner: fixture})
	if e.Passed || !reflect.DeepEqual(before, fixture.timers) || !slices.Contains(fixture.operations, "delete_timer") {
		t.Fatal(e, fixture)
	}
}

func TestClockTimerCleanupFailureFailsRun(t *testing.T) {
	fixture := newClockSimulation()
	fixture.cleanupFailed = true
	e := runClockTimer(context.Background(), Evidence{RunID: "synthetic-run", Passed: true, Metrics: map[string]any{}}, clockpack.Pack{Runner: fixture})
	if e.Passed {
		t.Fatal(e)
	}
}

func TestSiriStatusRejectsUnrecognizedOutput(t *testing.T) {
	if !validBridgeStatus([]byte("Apple Bridge background service is enabled.\n")) || validBridgeStatus([]byte("synthetic arbitrary output")) {
		t.Fatal("status validation failed")
	}
}
