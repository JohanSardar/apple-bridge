package contracts

type IPhoneCaptureAudioArgs struct {
	DeviceID string `json:"device_id"`
	Seconds  *int   `json:"seconds,omitempty"`
}

func IPhoneCaptureCommand(op string) bool {
	return op == "iphone.capture.photo" || op == "iphone.capture.audio"
}

func init() {
	registerDescriptors(func() []OperationDescriptor {
		return []OperationDescriptor{{Name: "iphone.capture.audio", Version: 1, Kind: CommandOp, Source: IPhone, Scope: "iphone:write", Capability: "iphone.capture.audio", Input: Command{}, Output: CommandRecord{}, Description: "Record up to ten seconds after on-phone Face ID approval and upload to the gateway"}}
	})
	RegisterIPhoneCommand("iphone.capture.audio", func(raw []byte) error {
		var args IPhoneCaptureAudioArgs
		if DecodeInput(raw, &args) != nil || args.Seconds != nil && (*args.Seconds < 1 || *args.Seconds > 10) {
			return ErrInvalid
		}
		return nil
	})
}
