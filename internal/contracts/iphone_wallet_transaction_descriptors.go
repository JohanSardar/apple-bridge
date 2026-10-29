package contracts

func init() {
	registerDescriptors(func() []OperationDescriptor {
		return []OperationDescriptor{{Name: "iphone.wallet.transaction", Version: 1, Kind: CommandOp, Source: IPhone, Scope: "iphone:write", Capability: "iphone.wallet.transaction", Input: Command{}, Output: CommandRecord{}, Description: "Apple-approval gated wallet transaction on the paired iPhone"}}
	})
	RegisterIPhoneCommand("iphone.wallet.transaction", func(raw []byte) error {
		var args struct {
			DeviceID string `json:"device_id"`
			Limit    *int   `json:"limit,omitempty"`
			Offset   *int   `json:"offset,omitempty"`
		}
		if err := DecodeInput(raw, &args); err != nil {
			return err
		}
		if (args.Limit != nil && (*args.Limit < 1 || *args.Limit > 50)) || (args.Offset != nil && (*args.Offset < 0 || *args.Offset > 100000)) {
			return ErrInvalid
		}
		return nil
	})
}
