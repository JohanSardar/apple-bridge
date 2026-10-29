package contracts

import (
	"github.com/google/uuid"
	"math"
	"time"
)

type IPhoneAlarmArgs struct {
	DeviceID        string   `json:"device_id"`
	ID              string   `json:"id,omitempty"`
	FireAt          float64  `json:"fire_at,omitempty"`
	DurationSeconds float64  `json:"duration_seconds,omitempty"`
	SnoozeSeconds   *float64 `json:"snooze_seconds,omitempty"`
	Title           string   `json:"title,omitempty"`
}

func registerAlarmCommand(name string) {
	RegisterIPhoneCommand(name, func(raw []byte) error {
		var a IPhoneAlarmArgs
		if err := DecodeInput(raw, &a); err != nil {
			return err
		}
		if a.SnoozeSeconds != nil && (*a.SnoozeSeconds <= 0 || math.IsInf(*a.SnoozeSeconds, 0) || math.IsNaN(*a.SnoozeSeconds)) {
			return ErrInvalid
		}
		if len(a.Title) > 200 {
			return ErrInvalid
		}
		switch name {
		case "iphone.alarm.create", "iphone.alarm.update":
			if a.FireAt <= float64(time.Now().Unix()) {
				return ErrInvalid
			}
		case "iphone.timer.start":
			if a.DurationSeconds <= 0 {
				return ErrInvalid
			}
		}
		switch name {
		case "iphone.alarm.update", "iphone.alarm.cancel", "iphone.timer.cancel":
			if _, err := uuid.Parse(a.ID); err != nil {
				return ErrInvalid
			}
		}
		if a.ID != "" {
			if _, err := uuid.Parse(a.ID); err != nil {
				return ErrInvalid
			}
		}
		return nil
	})
}
