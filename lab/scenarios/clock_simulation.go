package scenarios

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"slices"

	"github.com/JohanSardar/apple-bridge/internal/clockpack"
	"github.com/JohanSardar/apple-bridge/internal/shortcut"
)

type clockSimulation struct {
	alarms        []clockpack.Alarm
	timers        []clockpack.Timer
	recentTimers  []clockpack.Timer
	capabilities  []string
	operations    []string
	failStart     bool
	cleanupFailed bool
	dropAlarmName bool
}

func newClockSimulation() *clockSimulation {
	return &clockSimulation{
		alarms:       []clockpack.Alarm{{ID: "existing-alarm", Label: "synthetic baseline", Time: "08:00", RepeatDays: []int{}, Enabled: false}},
		timers:       []clockpack.Timer{{ID: "existing-timer", Label: "synthetic baseline", Duration: 600, Running: false}},
		recentTimers: []clockpack.Timer{{ID: "recent:1", Label: "synthetic recent", Duration: 300, Running: false}},
		capabilities: []string{"timers", "start_timer", "delete_timer"},
	}
}

func (f *clockSimulation) Run(ctx context.Context, input shortcut.Input) (shortcut.Result, error) {
	if err := ctx.Err(); err != nil {
		return shortcut.Result{}, err
	}
	var request clockpack.Request
	if json.Unmarshal(input.Data, &request) != nil {
		return shortcut.Result{}, errors.New("invalid synthetic request")
	}
	f.operations = append(f.operations, request.Operation)
	previousAlarms := slices.Clone(f.alarms)
	switch request.Operation {
	case "create_alarm":
		alarm := *request.Alarm
		alarm.ID = "created-alarm"
		if f.dropAlarmName {
			alarm.Label = "Alarm"
		}
		f.alarms = append(f.alarms, alarm)
	case "toggle_alarm":
		for i := range f.alarms {
			if f.alarms[i].ID == request.Alarm.ID {
				f.alarms[i] = *request.Alarm
			}
		}
	case "delete_alarm":
		f.alarms = slices.DeleteFunc(f.alarms, func(a clockpack.Alarm) bool { return a.ID == request.Alarm.ID })
	case "start_timer":
		f.timers = append(f.timers, clockpack.Timer{ID: "created-timer", Label: request.Label, Duration: request.Duration, Running: true})
		f.recentTimers = append(f.recentTimers, clockpack.Timer{ID: "recent:2", Label: request.Label, Duration: request.Duration, Running: false})
		if f.failStart {
			return shortcut.Result{}, errors.New("synthetic lost reply after creation")
		}
	case "delete_timer":
		if f.cleanupFailed {
			return shortcut.Result{}, errors.New("synthetic cleanup failure")
		}
		f.timers = slices.DeleteFunc(f.timers, func(t clockpack.Timer) bool { return t.ID == request.Timer.ID })
	case "delete_recent_timer":
		f.recentTimers = slices.DeleteFunc(f.recentTimers, func(t clockpack.Timer) bool { return t.ID == request.Timer.ID })
	case "alarms", "timers", "recent_timers", "capabilities":
	default:
		return shortcut.Result{}, fmt.Errorf("unsupported synthetic operation")
	}
	response := clockpack.Readback{Version: 1, Operation: request.Operation, Alarms: f.alarms, Timers: f.timers, Capabilities: f.capabilities, TimerRunning: request.Operation == "start_timer", Duration: request.Duration, Label: request.Label}
	if request.Operation == "recent_timers" || request.Operation == "delete_recent_timer" {
		response.Timers = f.recentTimers
	}
	if request.Operation == "toggle_alarm" || request.Operation == "delete_alarm" {
		response.Alarms = previousAlarms
	}
	data, err := json.Marshal(response)
	return shortcut.Result{Data: data}, err
}
