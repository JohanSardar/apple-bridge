package contracts

import (
	"bytes"
	"encoding/json"
	"os"
)

func CheckIPCFixtures(path string) (int, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return 0, err
	}
	var frames []json.RawMessage
	if err = json.Unmarshal(data, &frames); err != nil {
		return 0, err
	}
	for _, raw := range frames {
		var frame IPCFrame
		if err = json.Unmarshal(raw, &frame); err != nil {
			return 0, err
		}
		encoded, err := EncodeFrame(frame)
		if err != nil {
			return 0, err
		}
		var decoded IPCFrame
		if _, err = ReadFrame(bytes.NewReader(encoded), &decoded); err != nil || decoded.T != frame.T {
			return 0, ErrCorrupt
		}
		switch frame.T {
		case "Prepare":
			var value Prepare
			err = json.Unmarshal(raw, &value)
			if value.CommandID == "" {
				return 0, ErrInvalid
			}
		case "Prepared":
			var value Prepared
			err = json.Unmarshal(raw, &value)
			if value.Ticket == "" {
				return 0, ErrInvalid
			}
		case "Dispatch":
			var value Dispatch
			err = json.Unmarshal(raw, &value)
			if value.OwnerFence == 0 {
				return 0, ErrInvalid
			}
		case "Outcome":
			var value Outcome
			err = json.Unmarshal(raw, &value)
			if value.Disposition == "" {
				return 0, ErrInvalid
			}
		case "Resolve":
			var value Resolve
			err = json.Unmarshal(raw, &value)
			if value.CommandID == "" {
				return 0, ErrInvalid
			}
		}
		if err != nil {
			return 0, err
		}
	}
	return len(frames), nil
}
