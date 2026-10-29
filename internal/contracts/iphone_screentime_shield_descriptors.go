package contracts

func init() {
	registerDescriptors(func() []OperationDescriptor {
		return []OperationDescriptor{{Name: "iphone.screentime.shield", Version: 1, Kind: CommandOp, Source: IPhone, Scope: "iphone:write", Capability: "iphone.screentime.shield", Input: Command{}, Output: CommandRecord{}, Description: "Shield Bridge-owned Screen Time controls on the paired iPhone"}}
	})
	RegisterIPhoneCommand("iphone.screentime.shield", func(raw []byte) error {
		var args IPhoneTarget
		return DecodeInput(raw, &args)
	})
}
