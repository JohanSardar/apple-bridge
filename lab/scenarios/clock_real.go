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
	entries, err := (shortcut.CLI{}).Inventory(ctx, false, "")
	if err != nil {
		return limited("needs Shortcuts inventory access")
	}
	installed := false
	for _, entry := range entries {
		installed = installed || entry.Name == clockpack.Name
	}
	e.Metrics["pack_installed"] = installed
	if !installed {
		return limited("needs AB Clock import")
	}
	if s.ID == "CLK-TIMER-REAL" {
		_, err := pack.Exchange(ctx, clockpack.Request{Operation: "start_timer", Duration: 5, Label: "[ab-test] " + e.RunID})
		if err != nil {
			return limited("AB Clock did not return structured timer running, duration and label confirmation; macOS Start Timer action exists")
		}
		add("timer_running_duration_label_readback", true)
		return e
	}
	before, err := pack.Exchange(ctx, clockpack.Request{Operation: "alarms"})
	if err != nil {
		return limited("AB Clock did not return versioned structured alarm fields (id, time, label, repeat_days, enabled); mutations not attempted")
	}
	e.Metrics["alarms_before"] = len(before.Alarms)
	alarm := clockpack.Alarm{ID: e.RunID, Label: "[ab-test] " + e.RunID, Time: time.Now().Add(12 * time.Hour).Format("15:04"), RepeatDays: []int{}, Enabled: true}
	request := clockpack.Request{Alarm: &alarm}
	cleanup := func() {
		cleanupCtx, cancel := context.WithTimeout(context.Background(), 45*time.Second)
		defer cancel()
		request.Operation = "delete_alarm"
		_, deleteErr := pack.Exchange(cleanupCtx, request)
		after, readErr := pack.Exchange(cleanupCtx, clockpack.Request{Operation: "alarms"})
		add("cleanup_delete_readback", deleteErr == nil && readErr == nil && !clockAlarmPresent(after.Alarms, alarm.ID))
		if readErr == nil {
			e.Metrics["alarms_after"] = len(after.Alarms)
			add("cleanup_restores_count", len(after.Alarms) == len(before.Alarms))
		}
	}
	defer cleanup()
	request.Operation = "create_alarm"
	_, err = pack.Exchange(ctx, request)
	add("create_alarm_fields", err == nil)
	if err != nil {
		return e
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
