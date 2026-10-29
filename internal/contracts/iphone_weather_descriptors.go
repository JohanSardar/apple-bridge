package contracts

import "math"

func init() {
	registerDescriptors(func() []OperationDescriptor {
		return []OperationDescriptor{{Name: "iphone.weather", Version: 1, Kind: CommandOp, Source: IPhone, Scope: "iphone:write", Capability: "iphone.weather", Input: Command{}, Output: CommandRecord{}, Description: "Read opt-in weather from the paired iPhone"}}
	})
	RegisterIPhoneCommand("iphone.weather", func(raw []byte) error {
		var args struct {
			DeviceID  string   `json:"device_id"`
			Latitude  *float64 `json:"latitude"`
			Longitude *float64 `json:"longitude"`
		}
		if err := DecodeInput(raw, &args); err != nil {
			return err
		}
		if args.Latitude == nil || args.Longitude == nil || math.IsNaN(*args.Latitude) || math.IsNaN(*args.Longitude) || math.Abs(*args.Latitude) > 90 || math.Abs(*args.Longitude) > 180 {
			return ErrInvalid
		}
		return nil
	})
}
