package contracts

func init() {
	registerDescriptors(func() []OperationDescriptor {
		return []OperationDescriptor{{Name: "iphone.geofence.remove", Version: 1, Kind: CommandOp, Source: IPhone, Scope: "iphone:write", Capability: "iphone.geofence.remove", Input: Command{}, Output: CommandRecord{}, Description: "Remove a named geofence"}}
	})
	RegisterIPhoneCommand("iphone.geofence.remove", func(raw []byte) error {
		var args IPhoneGeofenceName
		if DecodeInput(raw, &args) != nil || !validFenceName(args.Name) {
			return ErrInvalid
		}
		return nil
	})
}
