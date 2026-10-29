package contracts

func init() {
	registerDescriptors(func() []OperationDescriptor {
		return []OperationDescriptor{{Name: "iphone.screentime.cancel", Version: 1, Kind: CommandOp, Source: IPhone, Scope: "iphone:write", Capability: "iphone.screentime.cancel", Input: Command{}, Output: CommandRecord{}, Description: "Cancel Bridge-owned Screen Time controls on the paired iPhone"}}
	})
	RegisterIPhoneCommand("iphone.screentime.cancel", func(raw []byte) error {
		var args IPhoneTarget
		return DecodeInput(raw, &args)
	})
}
