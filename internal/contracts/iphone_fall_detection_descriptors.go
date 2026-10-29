package contracts

func init() {
	registerDescriptors(func() []OperationDescriptor {
		return []OperationDescriptor{{Name: "iphone.fall.detection", Version: 1, Kind: CommandOp, Source: IPhone, Scope: "iphone:write", Capability: "iphone.fall.detection", Input: Command{}, Output: CommandRecord{}, Description: "Apple-approval gated fall detection on the paired iPhone"}}
	})
	RegisterIPhoneCommand("iphone.fall.detection", func(raw []byte) error {
		var args struct {
			DeviceID string `json:"device_id"`
		}
		if err := DecodeInput(raw, &args); err != nil {
			return err
		}

		return nil
	})
}
