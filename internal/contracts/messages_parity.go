package contracts

type MessagesParityArgs struct {
	PinnedIndex  *int   `json:"pinned_index,omitempty"`
	Muted        *bool  `json:"muted,omitempty"`
	ReadReceipts string `json:"read_receipts,omitempty"`
}

func MessagesParityFields(op string) []string {
	switch op {
	case "messages.pin":
		return []string{"id", "pinned_index"}
	case "messages.mute":
		return []string{"id", "muted"}
	case "messages.set_read_receipts":
		return []string{"id", "read_receipts"}
	case "messages.recover_chat", "messages.delete_chat_permanently":
		return []string{"id"}
	}
	return nil
}

func IsMessagesParity(op string) bool { return MessagesParityFields(op) != nil }

func ValidateMessagesParity(c Command, a MessagesArgs) error {
	if a.ID == "" || c.ExpectedVersion == 0 {
		return ErrInvalid
	}
	switch c.Op {
	case "messages.pin":
		if a.PinnedIndex == nil || *a.PinnedIndex < -1 || *a.PinnedIndex > 8 {
			return ErrInvalid
		}
	case "messages.mute":
		if a.Muted == nil {
			return ErrInvalid
		}
	case "messages.set_read_receipts":
		if a.ReadReceipts != "default" && a.ReadReceipts != "on" && a.ReadReceipts != "off" {
			return ErrInvalid
		}
	}
	return nil
}

func applyMessagesParitySchema(op string, command, args map[string]any) {
	fields := MessagesParityFields(op)
	if fields == nil {
		return
	}
	command["required"] = []string{"args", "expected_version"}
	command["properties"].(map[string]any)["expected_version"].(map[string]any)["minimum"] = 1
	args["required"] = fields
	properties := args["properties"].(map[string]any)
	for _, field := range fields {
		delete(properties[field].(map[string]any), "nullable")
	}
	if op == "messages.pin" {
		position := properties["pinned_index"].(map[string]any)
		position["minimum"], position["maximum"] = -1, 8
	}
	if op == "messages.set_read_receipts" {
		properties["read_receipts"].(map[string]any)["enum"] = []string{"on", "off", "default"}
	}
}
