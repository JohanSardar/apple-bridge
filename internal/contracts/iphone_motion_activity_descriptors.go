package contracts

func init() {
	registerDescriptors(func() []OperationDescriptor {
		return []OperationDescriptor{{Name: "iphone.motion.activity", Version: 1, Kind: CommandOp, Source: IPhone, Scope: "iphone:write", Capability: "iphone.motion.activity", Input: Command{}, Output: CommandRecord{}, Description: "Read authorized iPhone motion activity"}}
	})
	RegisterIPhoneCommand("iphone.motion.activity", func(raw []byte) error {
		var args IPhoneTarget
		return DecodeInput(raw, &args)
	})
}
