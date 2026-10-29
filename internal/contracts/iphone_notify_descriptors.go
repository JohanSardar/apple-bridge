package contracts

import "strings"

type IPhoneNotificationButton struct {
	ID    string `json:"id"`
	Title string `json:"title"`
}

type IPhoneNotifyArgs struct {
	DeviceID      string                     `json:"device_id"`
	Title         string                     `json:"title"`
	Body          string                     `json:"body"`
	Buttons       []IPhoneNotificationButton `json:"buttons,omitempty"`
	Thread        string                     `json:"thread,omitempty"`
	TimeSensitive bool                       `json:"time_sensitive,omitempty"`
}

func init() {
	registerDescriptors(func() []OperationDescriptor {
		return []OperationDescriptor{{Name: "iphone.notify", Version: 1, Kind: CommandOp, Source: IPhone, Scope: "iphone:write", Capability: "iphone.notify", Input: Command{}, Output: CommandRecord{}, Description: "Deliver a notification; with buttons, return the selected action as the signed command result"}}
	})
	RegisterIPhoneCommand("iphone.notify", func(raw []byte) error {
		var args IPhoneNotifyArgs
		if DecodeInput(raw, &args) != nil || strings.TrimSpace(args.Title) == "" || len(args.Title) > 200 || len(args.Body) > 1000 || len(args.Thread) > 100 || len(args.Buttons) > 4 {
			return ErrInvalid
		}
		seen := map[string]bool{}
		for _, button := range args.Buttons {
			if button.ID == "" || len(button.ID) > 64 || strings.HasPrefix(button.ID, "com.apple.") || strings.TrimSpace(button.Title) == "" || len(button.Title) > 100 || seen[button.ID] {
				return ErrInvalid
			}
			seen[button.ID] = true
		}
		return nil
	})
}
