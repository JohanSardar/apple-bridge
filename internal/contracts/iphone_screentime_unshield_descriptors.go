package contracts

func init() {
	registerDescriptors(func() []OperationDescriptor {
		return []OperationDescriptor{{Name: "iphone.screentime.unshield", Version: 1, Kind: CommandOp, Source: IPhone, Scope: "iphone:write", Capability: "iphone.screentime.unshield", Input: Command{}, Output: CommandRecord{}, Description: "Unshield Bridge-owned Screen Time controls on the paired iPhone"}}
	})
	RegisterIPhoneCommand("iphone.screentime.unshield", func(raw []byte) error {
		var args IPhoneTarget
		return DecodeInput(raw, &args)
	})
}
