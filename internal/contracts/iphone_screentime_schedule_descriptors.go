package contracts

func init() {
	registerDescriptors(func() []OperationDescriptor {
		return []OperationDescriptor{{Name: "iphone.screentime.schedule", Version: 1, Kind: CommandOp, Source: IPhone, Scope: "iphone:write", Capability: "iphone.screentime.schedule", Input: Command{}, Output: CommandRecord{}, Description: "Schedule Bridge-owned Screen Time controls on the paired iPhone"}}
	})
	RegisterIPhoneCommand("iphone.screentime.schedule", func(raw []byte) error {
		var args struct {
			DeviceID    string `json:"device_id"`
			StartMinute int    `json:"start_minute"`
			EndMinute   int    `json:"end_minute"`
		}
		if err := DecodeInput(raw, &args); err != nil {
			return err
		}
		if args.StartMinute < 0 || args.StartMinute >= 1440 || args.EndMinute < 0 || args.EndMinute >= 1440 || (args.EndMinute-args.StartMinute+1440)%1440 < 15 {
			return ErrInvalid
		}
		return nil
	})
}
