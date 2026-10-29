package contracts

func init() {
	registerDescriptors(func() []OperationDescriptor {
		return []OperationDescriptor{{Name: "iphone.location.current", Version: 1, Kind: CommandOp, Source: IPhone, Scope: "iphone:write", Capability: "iphone.location.current", Input: Command{}, Output: CommandRecord{}, Description: "Read precise location into an iPhone location record"}}
	})
	RegisterIPhoneCommand("iphone.location.current", func(raw []byte) error { var args IPhoneTarget; return DecodeInput(raw, &args) })
}
