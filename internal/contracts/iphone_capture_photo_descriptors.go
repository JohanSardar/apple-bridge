package contracts

func init() {
	registerDescriptors(func() []OperationDescriptor {
		return []OperationDescriptor{{Name: "iphone.capture.photo", Version: 1, Kind: CommandOp, Source: IPhone, Scope: "iphone:write", Capability: "iphone.capture.photo", Input: Command{}, Output: CommandRecord{}, Description: "Capture a photo after on-phone Face ID approval and upload to the gateway"}}
	})
	RegisterIPhoneCommand("iphone.capture.photo", func(raw []byte) error {
		var args IPhoneTarget
		return DecodeInput(raw, &args)
	})
}
