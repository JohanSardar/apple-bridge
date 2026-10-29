package contracts

import (
	"encoding/json"
	"strings"
)

const IPhone Source = "iphone"

func init() { RegisterSource(IPhone, "iPhone", true) }

type IPhoneTarget struct {
	DeviceID string `json:"device_id"`
}

var iphoneValidators = map[string]func([]byte) error{}

func RegisterIPhoneCommand(name string, validate func([]byte) error) {
	iphoneValidators[name] = validate
}

func ValidateIPhoneCommand(c Command) error {
	validate, ok := iphoneValidators[c.Op]
	var target IPhoneTarget
	if !ok || !ValidCommandID(c.ID) || c.WaitMS < 0 || c.WaitMS > 15000 || len(c.Args) > 1800 || json.Unmarshal(c.Args, &target) != nil || !strings.HasPrefix(target.DeviceID, "iphone:") {
		return ErrInvalid
	}
	return validate(c.Args)
}
