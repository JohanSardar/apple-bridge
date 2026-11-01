package scenarios

import (
	"context"
	"errors"
	"strings"
	"testing"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/shortcut"
)

func TestSiriProbeAllowsBoundedFirstRegistration(t *testing.T) {
	for _, tc := range []struct {
		id, level string
		want      time.Duration
	}{{"SIRI-INTENTS", "real", 5 * time.Minute}, {"SIRI-INTENTS", "sim", 20 * time.Second}, {"USE-ACTION-PROBE", "real", 20 * time.Second}} {
		fixture := &shortcutProbeSimulation{entries: []shortcut.Entry{{Name: "AB Bridge Status"}}, result: shortcut.Result{Data: []byte("Apple Bridge background service is not registered.")}}
		e := runShortcutProbe(context.Background(), Scenario{ID: tc.id}, tc.level, "AB Bridge Status", fixture, true, validBridgeStatus)
		if !e.Passed || fixture.runs != 1 || fixture.timeout != tc.want {
			t.Fatal(tc, e, fixture.runs, fixture.timeout)
		}
	}
}

func TestShortcutProbeRequiresQualifiedUniquePack(t *testing.T) {
	for _, count := range []int{0, 1, 2} {
		for _, qualified := range []bool{false, true} {
			fixture := &shortcutProbeSimulation{result: shortcut.Result{Data: []byte("synthetic"), ExitStatus: 0}}
			for i := 0; i < count; i++ {
				fixture.entries = append(fixture.entries, shortcut.Entry{Name: "AB Screen Time"})
			}
			e := runShortcutProbe(context.Background(), Scenario{}, "sim", "AB Screen Time", fixture, qualified, func([]byte) bool { return true })
			want := count == 1 && qualified
			if e.Passed != want || (fixture.runs == 1) != want {
				t.Fatal(count, qualified, e, fixture.runs)
			}
		}
	}
}

func TestShortcutProbeRejectsFailedAndEmptyOutput(t *testing.T) {
	for _, result := range []shortcut.Result{{}, {Data: []byte("synthetic"), ExitStatus: 1}, {Data: []byte("synthetic"), TimedOut: true}, {Data: make([]byte, (32<<10)+1)}} {
		fixture := &shortcutProbeSimulation{entries: []shortcut.Entry{{Name: "AB Screen Time"}}, result: result}
		if e := runShortcutProbe(context.Background(), Scenario{}, "sim", "AB Screen Time", fixture, true, func([]byte) bool { return true }); e.Passed {
			t.Fatal(result)
		}
	}
	fixture := &shortcutProbeSimulation{err: errors.New("private content")}
	e := runShortcutProbe(context.Background(), Scenario{}, "sim", "AB Screen Time", fixture, true, func([]byte) bool { return true })
	if e.Passed || fixture.runs != 0 || strings.Contains(e.Conditions["reason"], "private content") {
		t.Fatal(e)
	}
}
