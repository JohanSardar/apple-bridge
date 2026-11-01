package clockpack

import (
	"context"
	"encoding/json"
	"fmt"
	"slices"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/shortcut"
)

type Alarm struct {
	ID         string `json:"id"`
	Label      string `json:"label"`
	Time       string `json:"time"`
	RepeatDays []int  `json:"repeat_days"`
	Enabled    bool   `json:"enabled"`
}

type Timer struct {
	ID       string `json:"id"`
	Label    string `json:"label"`
	Duration int    `json:"duration"`
	Running  bool   `json:"running"`
}

type Request struct {
	Operation string `json:"operation"`
	Alarm     *Alarm `json:"alarm,omitempty"`
	Timer     *Timer `json:"timer,omitempty"`
	Duration  int    `json:"duration,omitempty"`
	Label     string `json:"label,omitempty"`
	City      string `json:"city,omitempty"`
}

type Readback struct {
	Version      int      `json:"version"`
	Capabilities []string `json:"capabilities"`
	Timers       []Timer  `json:"timers"`
	Operation    string   `json:"operation"`
	Alarms       []Alarm  `json:"alarms"`
	TimerRunning bool     `json:"timer_running"`
	Duration     int      `json:"duration"`
	Label        string   `json:"label"`
	City         string   `json:"city"`
	LocalTime    string   `json:"local_time"`
}

func validAlarm(a Alarm) bool {
	if a.ID == "" {
		return false
	}
	if _, err := time.Parse("15:04", a.Time); err != nil {
		return false
	}
	seen := map[int]bool{}
	for _, day := range a.RepeatDays {
		if day < 0 || day > 6 || seen[day] {
			return false
		}
		seen[day] = true
	}
	return true
}

func (p Pack) Exchange(ctx context.Context, request Request) (Readback, error) {
	var out Readback
	switch request.Operation {
	case "alarms", "timers", "capabilities":
	case "delete_timer":
		if request.Timer == nil || request.Timer.ID == "" || !strings.HasPrefix(request.Timer.Label, "[ab-test]") {
			return out, fmt.Errorf("valid tagged timer required")
		}
	case "create_alarm", "toggle_alarm", "delete_alarm":
		if request.Alarm == nil || !strings.HasPrefix(request.Alarm.Label, "[ab-test]") || !validAlarm(*request.Alarm) {
			return out, fmt.Errorf("valid tagged alarm required")
		}
	case "start_timer":
		if request.Duration < 1 || request.Duration > 86400 || !strings.HasPrefix(request.Label, "[ab-test]") {
			return out, fmt.Errorf("bounded tagged timer required")
		}
	case "world_time":
		if strings.TrimSpace(request.City) == "" || len(request.City) > 128 {
			return out, fmt.Errorf("city required")
		}
	default:
		return out, ErrUnqualified
	}
	if p.Runner == nil {
		return out, ErrUnqualified
	}
	data, err := json.Marshal(request)
	if err != nil {
		return out, err
	}
	timeout := p.Timeout
	if timeout == 0 {
		timeout = 15 * time.Second
	}
	result, err := p.Runner.Run(ctx, shortcut.Input{Shortcut: Name, Timeout: timeout, Data: data})
	if err != nil {
		return out, err
	}
	if result.TimedOut || result.ExitStatus != 0 || len(result.Data) > 32<<10 {
		return out, ErrUnqualified
	}
	if json.Unmarshal(result.Data, &out) != nil || out.Version != 1 || out.Operation != request.Operation {
		return Readback{}, ErrUnqualified
	}
	var fields struct {
		Alarms []map[string]json.RawMessage `json:"alarms"`
		Timers []map[string]json.RawMessage `json:"timers"`
	}
	if json.Unmarshal(result.Data, &fields) != nil {
		return Readback{}, ErrUnqualified
	}
	for _, alarm := range fields.Alarms {
		for _, field := range []string{"id", "label", "time", "repeat_days", "enabled"} {
			value, ok := alarm[field]
			if !ok || string(value) == "null" {
				return Readback{}, ErrUnqualified
			}
		}
	}
	for _, timer := range fields.Timers {
		for _, field := range []string{"id", "label", "duration", "running"} {
			value, ok := timer[field]
			if !ok || string(value) == "null" {
				return Readback{}, ErrUnqualified
			}
		}
	}

	ids := map[string]bool{}
	for i := range out.Alarms {
		a := &out.Alarms[i]
		if parsed, err := time.Parse("15:4", a.Time); err == nil {
			a.Time = parsed.Format("15:04")
		}
		if !validAlarm(*a) || ids[a.ID] {
			return Readback{}, ErrUnqualified
		}
		ids[a.ID] = true
		slices.Sort(a.RepeatDays)
	}
	switch request.Operation {
	case "capabilities":
		if out.Capabilities == nil {
			return Readback{}, ErrUnqualified
		}
	case "timers", "delete_timer":
		if out.Timers == nil {
			return Readback{}, ErrUnqualified
		}
		seen := map[string]bool{}
		for _, timer := range out.Timers {
			if timer.ID == "" || seen[timer.ID] || timer.Duration < 1 || timer.Duration > 86400 {
				return Readback{}, ErrUnqualified
			}
			seen[timer.ID] = true
			if request.Operation == "delete_timer" && timer.ID == request.Timer.ID {
				return Readback{}, ErrUnqualified
			}
		}
	case "alarms":
		if out.Alarms == nil {
			return Readback{}, ErrUnqualified
		}
	case "create_alarm", "toggle_alarm", "delete_alarm":
		if out.Alarms == nil {
			return Readback{}, ErrUnqualified
		}
		if !alarmMutationMatches(request, out.Alarms) {
			fresh, readErr := p.Exchange(ctx, Request{Operation: "alarms"})
			if readErr != nil {
				return out, ErrUnqualified
			}
			out.Alarms = fresh.Alarms
			if !alarmMutationMatches(request, out.Alarms) {
				return out, ErrUnqualified
			}
		}
	case "start_timer":
		if !out.TimerRunning || out.Duration != request.Duration || out.Label != request.Label {
			return Readback{}, ErrUnqualified
		}
	case "world_time":
		if out.City != request.City {
			return Readback{}, ErrUnqualified
		}
		if _, err := time.Parse(time.RFC3339, out.LocalTime); err != nil {
			return Readback{}, ErrUnqualified
		}
	}
	return out, nil
}

func alarmMutationMatches(request Request, alarms []Alarm) bool {
	found := false
	for _, alarm := range alarms {
		if request.Operation == "create_alarm" && alarm.Label != request.Alarm.Label || request.Operation != "create_alarm" && alarm.ID != request.Alarm.ID {
			continue
		}
		if found {
			return false
		}
		found = true
		expectedAlarm := *request.Alarm
		expectedAlarm.RepeatDays = slices.Clone(expectedAlarm.RepeatDays)
		slices.Sort(expectedAlarm.RepeatDays)
		if request.Operation == "create_alarm" {
			expectedAlarm.ID = alarm.ID
		}
		expected, _ := json.Marshal(expectedAlarm)
		actual, _ := json.Marshal(alarm)
		if string(expected) != string(actual) {
			return false
		}
	}
	return found == (request.Operation != "delete_alarm")
}
