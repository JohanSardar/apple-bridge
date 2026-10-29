package clockpack

import (
	"context"
	"encoding/json"
	"fmt"
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

type Request struct {
	Operation string `json:"operation"`
	Alarm     *Alarm `json:"alarm,omitempty"`
	Duration  int    `json:"duration,omitempty"`
	Label     string `json:"label,omitempty"`
	City      string `json:"city,omitempty"`
}

type Readback struct {
	Version      int     `json:"version"`
	Operation    string  `json:"operation"`
	Alarms       []Alarm `json:"alarms"`
	TimerRunning bool    `json:"timer_running"`
	Duration     int     `json:"duration"`
	Label        string  `json:"label"`
	City         string  `json:"city"`
	LocalTime    string  `json:"local_time"`
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
	case "alarms":
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
	result, err := p.Runner.Run(ctx, shortcut.Input{Shortcut: Name, Timeout: 15 * time.Second, Data: data})
	if err != nil {
		return out, err
	}
	if result.TimedOut || result.ExitStatus != 0 || len(result.Data) > 32<<10 {
		return out, ErrUnqualified
	}
	if json.Unmarshal(result.Data, &out) != nil || out.Version != 1 || out.Operation != request.Operation {
		return Readback{}, ErrUnqualified
	}
	ids := map[string]bool{}
	for _, a := range out.Alarms {
		if !validAlarm(a) || ids[a.ID] {
			return Readback{}, ErrUnqualified
		}
		ids[a.ID] = true
	}
	switch request.Operation {
	case "alarms":
		if out.Alarms == nil {
			return Readback{}, ErrUnqualified
		}
	case "create_alarm", "toggle_alarm", "delete_alarm":
		if out.Alarms == nil {
			return Readback{}, ErrUnqualified
		}
		found := false
		for _, a := range out.Alarms {
			if a.ID != request.Alarm.ID {
				continue
			}
			found = true
			expected, _ := json.Marshal(request.Alarm)
			actual, _ := json.Marshal(a)
			if string(expected) != string(actual) {
				return Readback{}, ErrUnqualified
			}
		}
		if found != (request.Operation != "delete_alarm") {
			return Readback{}, ErrUnqualified
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
