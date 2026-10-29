package contracts

func init() {
	registerDescriptors(func() []OperationDescriptor {
		return []OperationDescriptor{{Name: "iphone.device.status", Version: 1, Kind: CommandOp, Source: IPhone, Scope: "iphone:write", Capability: "iphone.device.status", Input: Command{}, Output: CommandRecord{}, Description: "Read iPhone device.status through the paired device"}}
	})
	RegisterIPhoneCommand("iphone.device.status", func(raw []byte) error {
		var args IPhoneTarget
		return DecodeInput(raw, &args)
	})
}
