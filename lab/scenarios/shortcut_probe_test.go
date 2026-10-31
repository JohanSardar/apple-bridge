package scenarios

import (
	"context"
	"errors"
	"strings"
	"testing"

	"github.com/JohanSardar/apple-bridge/internal/shortcut"
)

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
