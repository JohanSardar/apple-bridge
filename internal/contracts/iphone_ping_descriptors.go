package contracts

func init() {
	registerDescriptors(func() []OperationDescriptor {
		return []OperationDescriptor{{Name: "iphone.ping", Version: 1, Kind: CommandOp, Source: IPhone, Scope: "iphone:write", Capability: "iphone.ping", Input: Command{}, Output: CommandRecord{}, Description: "Round-trip a command through the paired iPhone and return its signed result"}}
	})
	RegisterIPhoneCommand("iphone.ping", func(raw []byte) error {
		var args IPhoneTarget
		return DecodeInput(raw, &args)
	})
}
