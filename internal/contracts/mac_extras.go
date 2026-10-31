package contracts

import "encoding/base64"

func ValidateMacExtrasQuery(q Query) error {
	if q.Op == "mac.recognize_song" {
		var args struct {
			AudioBlob string `json:"audio_blob"`
		}
		if DecodeInput(q.Args, &args) != nil {
			return ErrInvalid
		}
		data, err := base64.StdEncoding.DecodeString(args.AudioBlob)
		if err != nil || len(data) == 0 || len(data) > 8*1024*1024 {
			return ErrInvalid
		}
	} else {
		var args struct{}
		if string(q.Args) == "null" || DecodeInput(q.Args, &args) != nil {
			return ErrInvalid
		}
	}
	return nil
}

func MacExtrasQuery(op string) bool {
	return op == "mac.bluetooth_devices" || op == "mac.time_machine_status" || op == "mac.recognize_song"
}
