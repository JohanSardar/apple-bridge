package scenarios

import (
	"context"
	"reflect"
	"slices"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/clockpack"
)

func runClockTimer(ctx context.Context, initial Evidence, pack clockpack.Pack) (e Evidence) {
	e = initial
	add := func(name string, passed bool) {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: passed})
		e.Passed = e.Passed && passed
	}
	limited := func(reason string) Evidence {
		e.Status, e.Passed = "limited", false
		e.Conditions = map[string]string{"reason": reason}
		return e
	}
	capabilities, err := pack.Exchange(ctx, clockpack.Request{Operation: "capabilities"})
	qualified := err == nil
	for _, capability := range []string{"timers", "start_timer", "delete_timer"} {
		qualified = qualified && slices.Contains(capabilities.Capabilities, capability)
	}
	add("timer_readback_and_cleanup_qualified", qualified)
	if !qualified {
		return limited("Clock timer readback and cleanup are unavailable. Open Clock, allow the signed Apple Bridge Helper Accessibility access in System Settings, and retry. No timer was started.")
	}
	before, err := pack.Exchange(ctx, clockpack.Request{Operation: "timers"})
	add("timer_baseline_readback", err == nil)
	if err != nil {
		return e
	}
	label := "[ab-test] " + e.RunID
	baseline := map[string]clockpack.Timer{}
	for _, timer := range before.Timers {
		baseline[timer.ID] = timer
	}
	defer func() {
		cleanup, cancel := context.WithTimeout(context.Background(), 45*time.Second)
		defer cancel()
		observed, err := pack.Exchange(cleanup, clockpack.Request{Operation: "timers"})
		add("cleanup_timer_inventory", err == nil)
		if err != nil {
			return
		}
		for _, timer := range observed.Timers {
			if _, existed := baseline[timer.ID]; existed || timer.Label != label {
				continue
			}
			_, err := pack.Exchange(cleanup, clockpack.Request{Operation: "delete_timer", Timer: &timer})
			add("cleanup_timer_deleted", err == nil)
		}
		after, err := pack.Exchange(cleanup, clockpack.Request{Operation: "timers"})
		remaining := map[string]clockpack.Timer{}
		for _, timer := range after.Timers {
			remaining[timer.ID] = timer
		}
		add("cleanup_timer_baseline_restored", err == nil && reflect.DeepEqual(baseline, remaining))
	}()
	_, err = pack.Exchange(ctx, clockpack.Request{Operation: "start_timer", Duration: 120, Label: label})
	add("timer_running_duration_label_readback", err == nil)
	observed, readErr := pack.Exchange(ctx, clockpack.Request{Operation: "timers"})
	matches := 0
	for _, timer := range observed.Timers {
		if _, existed := baseline[timer.ID]; !existed && timer.Label == label && timer.Duration == 120 && timer.Running {
			matches++
		}
	}
	add("created_timer_list_readback", readErr == nil && matches == 1)
	return e
}
