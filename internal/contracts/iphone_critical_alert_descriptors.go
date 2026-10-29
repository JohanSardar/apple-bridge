package contracts

func init() {
	registerDescriptors(func() []OperationDescriptor {
		return []OperationDescriptor{{Name: "iphone.critical.alert", Version: 1, Kind: CommandOp, Source: IPhone, Scope: "iphone:write", Capability: "iphone.critical.alert", Input: Command{}, Output: CommandRecord{}, Description: "Apple-approval gated critical alert on the paired iPhone"}}
	})
	RegisterIPhoneCommand("iphone.critical.alert", func(raw []byte) error {
		var args struct {
			DeviceID string `json:"device_id"`
			Title    string `json:"title"`
			Body     string `json:"body"`
		}
		if err := DecodeInput(raw, &args); err != nil {
			return err
		}
		if len(args.Title) == 0 || len(args.Title) > 128 || len(args.Body) > 1024 {
			return ErrInvalid
		}
		return nil
	})
}
