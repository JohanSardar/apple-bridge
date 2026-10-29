package contracts

import (
	"math"
	"time"
)

func init() {
	registerDescriptors(func() []OperationDescriptor {
		return []OperationDescriptor{{Name: "iphone.health.log", Version: 1, Kind: CommandOp, Source: IPhone, Scope: "iphone:write", Capability: "iphone.health.log", Input: Command{}, Output: CommandRecord{}, Description: "Log tagged water (ml), weight (kg), mindful minutes or workout minutes on the paired iPhone"}}
	})
	RegisterIPhoneCommand("iphone.health.log", func(raw []byte) error {
		var args struct {
			DeviceID string  `json:"device_id"`
			Kind     string  `json:"kind"`
			Value    float64 `json:"value"`
			Start    string  `json:"start"`
			Tag      string  `json:"tag"`
			Activity string  `json:"activity,omitempty"`
		}
		if err := DecodeInput(raw, &args); err != nil {
			return err
		}
		if _, err := time.Parse(time.RFC3339, args.Start); err != nil {
			return ErrInvalid
		}
		if args.Value <= 0 || math.IsNaN(args.Value) || math.IsInf(args.Value, 0) || len(args.Tag) == 0 || len(args.Tag) > 128 {
			return ErrInvalid
		}
		switch args.Kind {
		case "water", "weight", "mindful":
		case "workout":
			switch args.Activity {
			case "walking", "running", "cycling", "other":
			default:
				return ErrInvalid
			}
		default:
			return ErrInvalid
		}
		return nil
	})
}
