package contracts

import "strings"

type IPhoneLiveArgs struct {
	DeviceID string  `json:"device_id"`
	TaskID   string  `json:"task_id"`
	Title    string  `json:"title"`
	Status   string  `json:"status"`
	Progress float64 `json:"progress"`
}

func registerIPhoneLive(op string) {
	registerDescriptors(func() []OperationDescriptor {
		return []OperationDescriptor{{Name: op, Version: 1, Kind: CommandOp, Source: IPhone, Scope: "iphone:write", Capability: op, Input: Command{}, Output: CommandRecord{}, Description: "Push running agent task status to the iPhone lock screen"}}
	})
	RegisterIPhoneCommand(op, func(raw []byte) error {
		var args IPhoneLiveArgs
		if DecodeInput(raw, &args) != nil || len(args.TaskID) == 0 || len(args.TaskID) > 128 || args.TaskID == "_start" || strings.TrimSpace(args.Title) == "" || len(args.Title) > 128 || len(args.Status) > 256 || args.Progress < 0 || args.Progress > 1 {
			return ErrInvalid
		}
		return nil
	})
}
