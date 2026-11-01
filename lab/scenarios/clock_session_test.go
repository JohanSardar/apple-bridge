package scenarios

import (
	"context"
	"encoding/json"
	"reflect"
	"slices"
	"testing"

	"github.com/JohanSardar/apple-bridge/internal/clockpack"
	"github.com/JohanSardar/apple-bridge/internal/shortcut"
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

func TestClockAlarmCleanupUsesReturnedIDWhenClockDropsTheLabel(t *testing.T) {
	fixture := newClockSimulation()
	fixture.dropAlarmName = true
	before := slices.Clone(fixture.alarms)
	e := runClockSession(context.Background(), Scenario{ID: "CLK-ALARM-REAL"}, Evidence{RunID: "synthetic-run", Passed: true, Metrics: map[string]any{}}, clockpack.Pack{Runner: fixture})
	if e.Passed || !reflect.DeepEqual(before, fixture.alarms) || !slices.Contains(fixture.operations, "delete_alarm") {
		t.Fatal(e, fixture.alarms, fixture.operations)
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

type changedAlarmBaseline struct{ *clockSimulation }

func (f changedAlarmBaseline) Run(ctx context.Context, input shortcut.Input) (shortcut.Result, error) {
	result, err := f.clockSimulation.Run(ctx, input)
	var request clockpack.Request
	if json.Unmarshal(input.Data, &request) == nil && request.Operation == "create_alarm" {
		f.alarms[0].Enabled = !f.alarms[0].Enabled
	}
	return result, err
}

func TestClockAlarmCleanupRejectsChangedBaselineWithSameCount(t *testing.T) {
	fixture := changedAlarmBaseline{newClockSimulation()}
	e := runClockSession(context.Background(), Scenario{ID: "CLK-ALARM-REAL"}, Evidence{RunID: "synthetic-run", Passed: true, Metrics: map[string]any{}}, clockpack.Pack{Runner: fixture})
	if e.Passed || len(fixture.alarms) != 1 {
		t.Fatal(e, fixture.alarms)
	}
	for _, assertion := range e.Assertions {
		if assertion.Name == "cleanup_restores_baseline" && !assertion.Passed {
			return
		}
	}
	t.Fatal("changed baseline was not detected", e)
}

func TestClockTimerCleanupPreservesPreexistingMatchingLabel(t *testing.T) {
	fixture := newClockSimulation()
	fixture.timers[0].Label = "[ab-test] synthetic-run"
	before := slices.Clone(fixture.timers)
	e := runClockTimer(context.Background(), Evidence{RunID: "synthetic-run", Passed: true, Metrics: map[string]any{}}, clockpack.Pack{Runner: fixture})
	if e.Passed || !reflect.DeepEqual(before, fixture.timers) || slices.Contains(fixture.operations, "start_timer") {
		t.Fatal(e, fixture.timers)
	}
}

func TestClockTimerRestoresRecentHistoryAfterLostReply(t *testing.T) {
	fixture := newClockSimulation()
	before := slices.Clone(fixture.recentTimers)
	fixture.failStart = true
	e := runClockTimer(context.Background(), Evidence{RunID: "synthetic-run", Passed: true, Metrics: map[string]any{}}, clockpack.Pack{Runner: fixture})
	if e.Passed || !reflect.DeepEqual(before, fixture.recentTimers) || !slices.Contains(fixture.operations, "delete_recent_timer") {
		t.Fatal(e, fixture.recentTimers)
	}
}
