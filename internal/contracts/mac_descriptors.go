package contracts

import "encoding/json"

type MacArgs struct {
	Text       *string  `json:"text,omitempty"`
	Volume     *float64 `json:"volume,omitempty"`
	Muted      *bool    `json:"muted,omitempty"`
	Device     string   `json:"device,omitempty"`
	Appearance string   `json:"appearance,omitempty"`
	BundleID   string   `json:"bundle_id,omitempty"`
	On         *bool    `json:"on,omitempty"`
	Duration   float64  `json:"duration,omitempty"`
}

func ValidateMacCommand(c Command) error {
	var a MacArgs
	if !ValidCommandID(c.ID) || c.WaitMS < 0 || c.WaitMS > 15000 || DecodeInput(c.Args, &a) != nil {
		return ErrInvalid
	}
	if c.Op != "mac.clipboard_write" && a.Text != nil {
		return ErrInvalid
	}
	switch c.Op {
	case "mac.clipboard_write":
		if a.Text == nil || len(*a.Text) > 65536 {
			return ErrInvalid
		}
	case "mac.set_volume":
		if a.Volume == nil || *a.Volume < 0 || *a.Volume > 100 {
			return ErrInvalid
		}
	case "mac.set_mute":
		if a.Muted == nil {
			return ErrInvalid
		}
	case "mac.set_output_device":
		if a.Device == "" {
			return ErrInvalid
		}
	case "mac.set_appearance":
		if a.Appearance != "light" && a.Appearance != "dark" {
			return ErrInvalid
		}
	case "mac.launch_app", "mac.activate_app", "mac.quit_app":
		if a.BundleID == "" || len(a.BundleID) > 255 {
			return ErrInvalid
		}
	case "mac.keep_awake":
		if a.On == nil || a.Duration < 0 || a.Duration > 86400 {
			return ErrInvalid
		}
	default:
		return ErrInvalid
	}
	return nil
}
func MacCompletionEvidence(raw json.RawMessage, hash string) bool {
	var e struct {
		Observed    bool            `json:"observed"`
		Fingerprint string          `json:"fingerprint"`
		Readback    json.RawMessage `json:"readback"`
	}
	return json.Unmarshal(raw, &e) == nil && e.Observed && hash != "" && e.Fingerprint == hash && len(e.Readback) > 0 && string(e.Readback) != "null"
}

func MacDescriptors() []OperationDescriptor {
	var out []OperationDescriptor
	for _, name := range []string{"mac.set_volume", "mac.set_mute", "mac.set_output_device", "mac.set_appearance", "mac.launch_app", "mac.activate_app", "mac.quit_app", "mac.keep_awake", "mac.clipboard_write"} {
		out = append(out, OperationDescriptor{Name: name, Version: 1, Kind: CommandOp, Source: "mac", Scope: "mac:write", Capability: name, Input: Command{}, Output: CommandRecord{}, Description: "Change Mac state with native readback"})
	}
	out = append(out, OperationDescriptor{Name: "mac.clipboard_read", Version: 1, Kind: QueryOp, Source: "mac", Scope: "mac:read", Capability: "mac.clipboard_read", Input: Query{}, Output: QueryResult{}, Description: "Read transient clipboard text without capturing contents"})
	return out
}

func init() {
	RegisterSource("mac", "Mac", true)
	registerDescriptors(MacDescriptors, 96, 97, 98, 99, 100, 101, 102, 103, 104, 105)
	for collection, prefix := range map[string]string{"mac.audio_device": "mad", "mac.display": "mdp", "mac.power": "mpw", "mac.network": "mnw", "mac.app": "map"} {
		Collections[collection] = prefix
	}
	messagesShape["mac.audio_device"] = map[string]fieldShape{"name": requiredField(shapeString), "input": requiredField(shapeBool), "output": requiredField(shapeBool), "default": requiredField(shapeBool), "volume": optionalField(shapeNumber), "muted": optionalField(shapeBool)}
	messagesShape["mac.display"] = map[string]fieldShape{"appearance": requiredField(shapeString), "displays": requiredField(shapeArray)}
	messagesShape["mac.power"] = map[string]fieldShape{"battery_percent": optionalField(shapeNumber), "charging": requiredField(shapeBool), "source": requiredField(shapeString), "low_power_mode": requiredField(shapeBool), "time_remaining": optionalField(shapeNumber), "keep_awake_held": requiredField(shapeBool)}
	messagesShape["mac.network"] = map[string]fieldShape{"ssid": optionalField(shapeString), "wifi_power": optionalField(shapeBool), "connected": requiredField(shapeBool), "interface": optionalField(shapeString), "vpn": requiredField(shapeBool)}
	messagesShape["mac.app"] = map[string]fieldShape{"bundle_id": requiredField(shapeString), "name": requiredField(shapeString), "running": requiredField(shapeBool), "frontmost": requiredField(shapeBool), "hidden": requiredField(shapeBool)}
}

func RedactMacCommand(c Command) Command {
	if c.Op == "mac.clipboard_write" {
		c.Args = json.RawMessage(`{}`)
	}
	return c
}

func macSchema(d OperationDescriptor, schema map[string]any) map[string]any {
	args := JSONSchema(MacArgs{})
	fields := args["properties"].(map[string]any)
	key := map[string]string{"mac.set_volume": "volume", "mac.set_mute": "muted", "mac.set_output_device": "device", "mac.set_appearance": "appearance", "mac.launch_app": "bundle_id", "mac.activate_app": "bundle_id", "mac.quit_app": "bundle_id", "mac.keep_awake": "on", "mac.clipboard_write": "text"}[d.Name]
	for name := range fields {
		if name != key && !(d.Name == "mac.keep_awake" && name == "duration") {
			delete(fields, name)
		}
	}
	field := fields[key].(map[string]any)
	delete(field, "nullable")
	switch key {
	case "volume":
		field["minimum"], field["maximum"] = 0, 100
	case "appearance":
		field["enum"] = []string{"light", "dark"}
	case "device", "bundle_id":
		field["minLength"] = 1
	case "text":
		field["maxLength"], field["description"] = 65536, "Transient clipboard text; at most 65536 UTF-8 bytes. Contents are redacted from durable command history."
	}
	if key == "bundle_id" {
		field["maxLength"] = 255
	}
	if duration, ok := fields["duration"].(map[string]any); ok {
		duration["minimum"], duration["maximum"] = 0, 86400
	}
	args["required"] = []string{key}
	properties := schema["properties"].(map[string]any)
	properties["args"] = args
	properties["wait_ms"].(map[string]any)["minimum"] = 0
	properties["wait_ms"].(map[string]any)["maximum"] = 15000
	schema["required"] = []string{"args"}
	return schema
}

func init() {
	registerDescriptors(func() []OperationDescriptor {
		return []OperationDescriptor{
			{Name: "mac.screen_record", Version: 1, Kind: CommandOp, Source: "mac", Scope: "mac:write", Capability: "mac.screen_record", Input: Command{}, Output: CommandRecord{}, Description: "Record up to 30 seconds of screen video as a blob"},
		}
	})
}
