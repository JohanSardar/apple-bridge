package contracts

func init() {
	registerDescriptors(func() []OperationDescriptor {
		return []OperationDescriptor{{Name: "iphone.location.push", Version: 1, Kind: CommandOp, Source: IPhone, Scope: "iphone:write", Capability: "iphone.location.push", Input: Command{}, Output: CommandRecord{}, Description: "Apple-approval gated location push on the paired iPhone"}}
	})
	RegisterIPhoneCommand("iphone.location.push", func(raw []byte) error {
		var args struct {
			DeviceID string `json:"device_id"`
		}
		if err := DecodeInput(raw, &args); err != nil {
			return err
		}

		return nil
	})
}
