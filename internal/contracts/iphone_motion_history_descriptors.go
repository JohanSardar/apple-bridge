package contracts

func init() {
	registerDescriptors(func() []OperationDescriptor {
		return []OperationDescriptor{{Name: "iphone.motion.history", Version: 1, Kind: CommandOp, Source: IPhone, Scope: "iphone:write", Capability: "iphone.motion.history", Input: Command{}, Output: CommandRecord{}, Description: "Read authorized iPhone motion history"}}
	})
	RegisterIPhoneCommand("iphone.motion.history", func(raw []byte) error {
		var args struct {
			DeviceID string `json:"device_id"`
			Days     int    `json:"days"`
		}
		if err := DecodeInput(raw, &args); err != nil {
			return err
		}
		if args.Days < 1 || args.Days > 7 {
			return ErrInvalid
		}
		return nil
	})
}
