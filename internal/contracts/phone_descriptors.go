package contracts

import "regexp"

var dialNumber = regexp.MustCompile(`^\+?[0-9]{3,15}$`)

type PhoneDialArgs struct {
	Number string `json:"number"`
}

func ValidatePhoneDial(c Command) error {
	var a PhoneDialArgs
	if !ValidCommandID(c.ID) || c.WaitMS < 0 || c.WaitMS > 15000 || DecodeInput(c.Args, &a) != nil || !dialNumber.MatchString(a.Number) {
		return ErrInvalid
	}
	return nil
}

func init() {
	registerDescriptors(func() []OperationDescriptor {
		return []OperationDescriptor{{Name: "phone.dial", Version: 1, Kind: CommandOp, Source: Phone, Scope: "phone:write", Capability: "phone.dial", Input: Command{}, Output: CommandRecord{}, Description: "Dial through Continuity with Mac confirmation; never retry an indeterminate dial"}}
	})
}
