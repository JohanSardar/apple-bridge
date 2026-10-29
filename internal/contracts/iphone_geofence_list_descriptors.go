package contracts

func init() {
	registerDescriptors(func() []OperationDescriptor {
		return []OperationDescriptor{{Name: "iphone.geofence.list", Version: 1, Kind: CommandOp, Source: IPhone, Scope: "iphone:write", Capability: "iphone.geofence.list", Input: Command{}, Output: CommandRecord{}, Description: "Publish named geofences as iPhone location records"}}
	})
	RegisterIPhoneCommand("iphone.geofence.list", func(raw []byte) error { var args IPhoneTarget; return DecodeInput(raw, &args) })
}
