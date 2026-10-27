package simapple

import (
	"encoding/json"
	"strings"
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

func helperMessageDefaults() map[string]any {
	return map[string]any{"kind": "message", "decode_status": "decoded", "error_code": 0, "edit_history": []any{}, "retracted_parts": []any{}, "formatting": []any{},
		"audio": map[string]any{"is_audio_message": false, "expirable": false}, "delivery": map[string]any{"quietly": false, "notified_anyway": false},
		"is_spam": false, "has_unseen_mention": false, "via_satellite": false, "is_sos": false}
}

func fill(data, defaults map[string]any) {
	for field, value := range defaults {
		if _, ok := data[field]; !ok {
			data[field] = value
		}
	}
}

func (f *fakeApple) observationFacts(collection string, data map[string]any) {
	switch strings.TrimPrefix(collection, "messages.") {
	case "message":
		f.messageFacts(data)
	case "chat":
		fill(data, map[string]any{"display_name": "", "capabilities": []any{}, "filter": "known", "is_archived": false, "is_spam": false, "read_receipts": "default"})
	case "attachment":
		fill(data, map[string]any{"genmoji": false, "hidden": false, "is_sticker": false, "uti": "public.data"})
	case "reaction":
		fill(data, map[string]any{"sent_at": time.Now().UTC()})
	}
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
		service, _ := chat["service"].(string)
		if service == "" {
			service = "iMessage"
		}
		data["service"] = service
	}
	fill(data, helperMessageDefaults())
	if _, ok := data["parts"]; !ok {
		data["parts"] = []any{}
		if text, ok := data["text"].(string); ok && text != "" {
			data["parts"] = []any{map[string]any{"index": 0, "kind": "text", "text": text}}
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
