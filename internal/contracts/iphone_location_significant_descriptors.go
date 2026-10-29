package contracts

func init() {
	registerDescriptors(func() []OperationDescriptor {
		return []OperationDescriptor{{Name: "iphone.location.significant", Version: 1, Kind: CommandOp, Source: IPhone, Scope: "iphone:write", Capability: "iphone.location.significant", Input: Command{}, Output: CommandRecord{}, Description: "Enable or disable significant location change records"}}
	})
	RegisterIPhoneCommand("iphone.location.significant", func(raw []byte) error {
		var args IPhoneSignificant
		if DecodeInput(raw, &args) != nil || args.Enabled == nil {
			return ErrInvalid
		}
		return nil
	})
}
