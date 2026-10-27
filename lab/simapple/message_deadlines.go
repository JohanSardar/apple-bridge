package simapple

import (
	"encoding/json"
	"time"
)

const (
	editWindow   = 15 * time.Minute
	unsendWindow = 2 * time.Minute
	editLimit    = 5
)

func simTime(value any) (time.Time, bool) {
	switch v := value.(type) {
	case time.Time:
		return v, !v.IsZero()
	case string:
		at, err := time.Parse(time.RFC3339Nano, v)
		return at, err == nil
	}
	return time.Time{}, false
}

func (f *fakeApple) messageFacts(data map[string]any) {
	if _, ok := data["service"].(string); !ok {
		raw, _ := json.Marshal(data["chat_id"])
		var ref struct {
			Ref struct {
				NativeKey string `json:"native_key"`
			} `json:"$ref"`
		}
		json.Unmarshal(raw, &ref)
		var chat map[string]any
		json.Unmarshal(f.records[ref.Ref.NativeKey].Data, &chat)
		if service, ok := chat["service"].(string); ok && service != "" {
			data["service"] = service
		}
	}
	delete(data, "edit_until")
	delete(data, "unsend_until")
	sent, ok := simTime(data["sent_at"])
	if !ok || data["service"] != "iMessage" || data["is_from_me"] != true || data["kind"] == "group_event" {
		return
	}
	if history, _ := data["edit_history"].([]any); len(history) < editLimit {
		data["edit_until"] = sent.Add(editWindow)
	}
	data["unsend_until"] = sent.Add(unsendWindow)
}

func actionWindowOpen(data map[string]any, edit bool, now time.Time) bool {
	field := "unsend_until"
	if edit {
		field = "edit_until"
	}
	sent, sentOK := simTime(data["sent_at"])
	until, ok := simTime(data[field])
	return sentOK && ok && !now.Before(sent) && now.Before(until)
}
