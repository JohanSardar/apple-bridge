package contracts

func init() {
	registerDescriptors(func() []OperationDescriptor {
		return []OperationDescriptor{{Name: "iphone.geofence.add", Version: 1, Kind: CommandOp, Source: IPhone, Scope: "iphone:write", Capability: "iphone.geofence.add", Input: Command{}, Output: CommandRecord{}, Description: "Monitor a named geofence"}}
	})
	RegisterIPhoneCommand("iphone.geofence.add", func(raw []byte) error { return validateIPhoneFence(raw) })
}
