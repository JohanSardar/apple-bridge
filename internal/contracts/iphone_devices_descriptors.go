package contracts

func init() {
	registerDescriptors(func() []OperationDescriptor {
		return []OperationDescriptor{{Name: "iphone.devices", Version: 1, Kind: ReadOp, Source: IPhone, Scope: "iphone:read", Input: struct{}{}, Output: []IPhoneDevice{}, Description: "List paired iPhones; revoke a device with the gateway revoke command"}}
	})
}

type IPhoneDevice struct {
	ID         string `json:"id"`
	GatewayKey []byte `json:"gateway_key,omitempty"`
}
