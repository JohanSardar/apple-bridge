package contracts

func init() {
	registerDescriptors(func() []OperationDescriptor {
		return []OperationDescriptor{{Name: "iphone.nfc.read", Version: 1, Kind: CommandOp, Source: IPhone, Scope: "iphone:write", Capability: "iphone.nfc.read", Input: Command{}, Output: CommandRecord{}, Description: "Read iPhone nfc.read through the paired device"}}
	})
	RegisterIPhoneCommand("iphone.nfc.read", func(raw []byte) error {
		var args IPhoneTarget
		return DecodeInput(raw, &args)
	})
}
