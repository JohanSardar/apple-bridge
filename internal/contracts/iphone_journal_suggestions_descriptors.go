package contracts

func init() {
	registerDescriptors(func() []OperationDescriptor {
		return []OperationDescriptor{{Name: "iphone.journal.suggestions", Version: 1, Kind: CommandOp, Source: IPhone, Scope: "iphone:write", Capability: "iphone.journal.suggestions", Input: Command{}, Output: CommandRecord{}, Description: "Read opt-in journal suggestions from the paired iPhone"}}
	})
	RegisterIPhoneCommand("iphone.journal.suggestions", func(raw []byte) error {
		var args struct {
			DeviceID string `json:"device_id"`
		}
		if err := DecodeInput(raw, &args); err != nil {
			return err
		}
		return nil
	})
}
