package contracts

import "github.com/google/uuid"

func init() {
	registerDescriptors(func() []OperationDescriptor {
		return []OperationDescriptor{{Name: "iphone.energy.hints", Version: 1, Kind: CommandOp, Source: IPhone, Scope: "iphone:write", Capability: "iphone.energy.hints", Input: Command{}, Output: CommandRecord{}, Description: "Read opt-in energy hints from the paired iPhone"}}
	})
	RegisterIPhoneCommand("iphone.energy.hints", func(raw []byte) error {
		var args struct {
			DeviceID string `json:"device_id"`
			VenueID  string `json:"venue_id"`
		}
		if err := DecodeInput(raw, &args); err != nil {
			return err
		}
		if _, err := uuid.Parse(args.VenueID); err != nil {
			return ErrInvalid
		}
		return nil
	})
}
