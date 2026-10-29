package contracts

import (
	"encoding/json"
	"time"
)

type FocusSetArgs struct {
	Mode  string `json:"mode"`
	On    *bool  `json:"on"`
	Until string `json:"until,omitempty"`
}

func ValidateFocusSet(c Command) error {
	var a FocusSetArgs
	if !ValidCommandID(c.ID) || c.WaitMS < 0 || c.WaitMS > 15000 || DecodeInput(c.Args, &a) != nil || a.On == nil || a.Mode != "Do Not Disturb" {
		return ErrInvalid
	}
	if a.Until != "" {
		until, err := time.Parse(time.RFC3339, a.Until)
		if err != nil || !*a.On || !until.After(time.Now()) {
			return ErrInvalid
		}
	}
	return nil
}

func init() {
	RegisterSource("focus", "Focus", true)
	Collections["focus.status"] = "fcs"
	messagesShape["focus.status"] = map[string]fieldShape{"focused": nullableField(shapeBool), "name": nullableField(shapeString), "authorization": requiredField(shapeString)}
	registerDescriptors(func() []OperationDescriptor {
		return []OperationDescriptor{
			{Name: "focus.set", Version: 1, Kind: CommandOp, Source: "focus", Scope: "focus:write", Capability: "focus.set", Input: Command{}, Output: CommandRecord{}, Description: "Set Do Not Disturb through AB Focus and confirm Focus status; agents require approval"},
		}
	}, 66, 90)
}

func FocusSetEvidence(raw json.RawMessage, hash string, args json.RawMessage) bool {
	var a FocusSetArgs
	var e struct {
		Observed    bool   `json:"observed"`
		Fingerprint string `json:"fingerprint"`
		Readback    struct {
			Focused       *bool  `json:"focused"`
			Authorization string `json:"authorization"`
		} `json:"readback"`
		Prepared struct {
			RequestHash string `json:"request_hash"`
		} `json:"prepared"`
	}
	return json.Unmarshal(args, &a) == nil && a.On != nil && json.Unmarshal(raw, &e) == nil && e.Observed && hash != "" && e.Fingerprint == hash && e.Prepared.RequestHash == hash && e.Readback.Authorization == "authorized" && e.Readback.Focused != nil && *e.Readback.Focused == *a.On
}
