package scenarios

import (
	"context"
	"reflect"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/clockpack"
	"github.com/JohanSardar/apple-bridge/internal/shortcut"
)

func runClockReal(ctx context.Context, s Scenario, initial Evidence, pack clockpack.Pack) (e Evidence) {
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
	app, stop, hostErr := startBridgeIntentHost(ctx)
	add("current_helper_intent_host_ready", hostErr == nil)
	if hostErr != nil {
		return e
	}
	defer stop()
	pack.Timeout = 5 * time.Minute
	if s.ID == "CLK-TIMER-REAL" {
		pack.Runner = clockTimerNativeRunner{app: app}
		return runClockSession(ctx, s, e, pack)
	}
	entries, err := (shortcut.CLI{}).Inventory(ctx, false, "")
	add("clock_inventory_available", err == nil)
	if err != nil {
		return limited("Shortcuts inventory could not be read. Open Shortcuts, verify its library is available, then retry. No Clock changes were made.")
	}
	matches := 0
	for _, entry := range entries {
		if entry.Name == clockpack.Name {
			matches++
		}
	}
	e.Metrics["pack_matches"] = matches
	add("unique_clock_pack_installed", matches == 1)
	if matches != 1 {
		return limited("Install exactly one AB Clock shortcut before retrying. No Clock changes were made.")
	}
	return runClockSession(ctx, s, e, pack)
}

func runClockSession(ctx context.Context, s Scenario, initial Evidence, pack clockpack.Pack) (e Evidence) {
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
	if s.ID == "CLK-TIMER-REAL" {
		return runClockTimer(ctx, e, pack)
	}

	before, err := pack.Exchange(ctx, clockpack.Request{Operation: "alarms"})
	add("alarm_baseline_readback", err == nil)
	if err != nil {
		return limited("AB Clock did not return structured alarm fields. Open the installed AB Clock in Shortcuts and verify alarm listing succeeds with id, time, label, repeat_days and enabled fields before retrying. No alarm was changed.")
	}
	e.Metrics["alarms_before"] = len(before.Alarms)
	alarm := clockpack.Alarm{ID: e.RunID, Label: "[ab-test] " + e.RunID, Time: time.Now().Add(12 * time.Hour).Format("15:04"), RepeatDays: []int{1, 3}, Enabled: true}
	for _, existing := range before.Alarms {
		if existing.Label == alarm.Label {
			return limited("A matching test alarm already exists. Remove only that test alarm in Clock, then retry.")
		}
	}
	baseline := map[string]bool{}
	for _, existing := range before.Alarms {
		baseline[existing.ID] = true
	}
	request := clockpack.Request{Alarm: &alarm}
	cleanup := func() {
		cleanupCtx, cancel := context.WithTimeout(context.Background(), 5*time.Minute)
		defer cancel()
		observed, readErr := pack.Exchange(cleanupCtx, clockpack.Request{Operation: "alarms"})
		if readErr != nil {
			add("cleanup_inventory", false)
			return
		}
		for _, candidate := range observed.Alarms {
			if candidate.Label == alarm.Label && !baseline[candidate.ID] {
				alarm = candidate
				break
			}
		}
		request.Operation = "delete_alarm"
		_, deleteErr := pack.Exchange(cleanupCtx, request)
		after, readErr := pack.Exchange(cleanupCtx, clockpack.Request{Operation: "alarms"})
		add("cleanup_delete_readback", deleteErr == nil && readErr == nil && !clockAlarmPresent(after.Alarms, alarm.ID))
		if readErr == nil {
			e.Metrics["alarms_after"] = len(after.Alarms)
			add("cleanup_restores_baseline", sameClockAlarms(after.Alarms, before.Alarms))
		}
	}
	defer cleanup()
	request.Operation = "create_alarm"
	created, err := pack.Exchange(ctx, request)
	add("create_alarm_fields", err == nil)
	if err != nil {
		return e
	}
	for _, candidate := range created.Alarms {
		if candidate.Label == alarm.Label {
			alarm.ID = candidate.ID
		}
	}
	readback, err := pack.Exchange(ctx, clockpack.Request{Operation: "alarms"})
	add("created_alarm_list_fields", err == nil && clockAlarmMatches(readback.Alarms, alarm))
	alarm.Enabled = false
	request.Operation = "toggle_alarm"
	_, err = pack.Exchange(ctx, request)
	add("toggle_alarm_fields", err == nil)
	readback, err = pack.Exchange(ctx, clockpack.Request{Operation: "alarms"})
	add("toggled_alarm_list_fields", err == nil && clockAlarmMatches(readback.Alarms, alarm))
	return e
}

func clockAlarmPresent(alarms []clockpack.Alarm, id string) bool {
	for _, alarm := range alarms {
		if alarm.ID == id {
			return true
		}
	}
	return false
}

func clockAlarmMatches(alarms []clockpack.Alarm, expected clockpack.Alarm) bool {
	for _, alarm := range alarms {
		if alarm.ID == expected.ID {
			return reflect.DeepEqual(alarm, expected)
		}
	}
	return false
}

func sameClockAlarms(actual, expected []clockpack.Alarm) bool {
	if len(actual) != len(expected) {
		return false
	}
	for _, alarm := range expected {
		if !clockAlarmMatches(actual, alarm) {
			return false
		}
	}
	return true
}
