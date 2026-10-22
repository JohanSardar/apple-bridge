package simapple

import (
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"sort"
	"time"
)

func (f *fakeApple) messageParityValidate(a contracts.NativeAction, data map[string]any) *contracts.Error {
	if !contracts.IsMessagesParity(a.Op) {
		return nil
	}
	if a.Op == "messages.pin" && (a.Messages.PinnedIndex == nil || *a.Messages.PinnedIndex < -1 || *a.Messages.PinnedIndex > 8) {
		return contracts.ErrInvalid
	}
	if a.Op == "messages.mute" && a.Messages.Muted == nil {
		return contracts.ErrInvalid
	}
	if a.Op == "messages.set_read_receipts" && a.Messages.ReadReceipts != "on" && a.Messages.ReadReceipts != "off" && a.Messages.ReadReceipts != "default" {
		return contracts.ErrInvalid
	}
	if f.records[a.NativeKey].Collection != "messages.chat" {
		return contracts.ErrInvalid
	}
	deleted := data["deleted_at"] != nil && data["deleted_at"] != ""
	recovery := a.Op == "messages.recover_chat" || a.Op == "messages.delete_chat_permanently"
	if deleted != recovery {
		return contracts.ErrConflict
	}
	if a.Op == "messages.set_read_receipts" && data["service"] != "iMessage" {
		return contracts.ErrUnsupported
	}
	if a.Op == "messages.pin" && *a.Messages.PinnedIndex >= 0 {
		pins := f.messagePins("")
		if len(pins) >= 9 && data["pinned_index"] == nil {
			return contracts.ErrConflict
		}
	}
	return nil
}

type messagePin struct {
	key   string
	index float64
}

func (f *fakeApple) messagePins(exclude string) []messagePin {
	pins := []messagePin{}
	for key, o := range f.records {
		if key == exclude || o.Collection != "messages.chat" || o.Kind == "absent" {
			continue
		}
		var d map[string]any
		json.Unmarshal(o.Data, &d)
		if index, ok := d["pinned_index"].(float64); ok && d["deleted_at"] == nil {
			pins = append(pins, messagePin{key, index})
		}
	}
	sort.Slice(pins, func(i, j int) bool {
		if pins[i].index == pins[j].index {
			return pins[i].key < pins[j].key
		}
		return pins[i].index < pins[j].index
	})
	return pins
}

func (f *fakeApple) messageBefore(a contracts.NativeAction) json.RawMessage {
	if a.Op != "messages.pin" {
		return f.records[a.NativeKey].Data
	}
	snapshot := map[string]json.RawMessage{"target": f.records[a.NativeKey].Data}
	for _, pin := range f.messagePins("") {
		snapshot["pin:"+pin.key] = f.records[pin.key].Data
	}
	raw, _ := json.Marshal(snapshot)
	return raw
}

func (f *fakeApple) messageParityApply(a contracts.NativeAction, _ time.Time, put func(string, string, map[string]any, bool)) {
	var d map[string]any
	json.Unmarshal(f.records[a.NativeKey].Data, &d)
	switch a.Op {
	case "messages.mute":
		d["muted"] = *a.Messages.Muted
		d["muted_until"] = nil
	case "messages.set_read_receipts":
		d["read_receipts"] = a.Messages.ReadReceipts
	case "messages.pin":
		pins := f.messagePins(a.NativeKey)
		index := *a.Messages.PinnedIndex
		d["pinned_index"] = nil
		if index >= 0 {
			if index > len(pins) {
				index = len(pins)
			}
			pins = append(pins, messagePin{})
			copy(pins[index+1:], pins[index:])
			pins[index] = messagePin{a.NativeKey, float64(index)}
		}
		for i, pin := range pins {
			var p map[string]any
			json.Unmarshal(f.records[pin.key].Data, &p)
			p["pinned_index"] = i
			if pin.key == a.NativeKey {
				d = p
			} else {
				put("chat", pin.key, p, false)
			}
		}
	case "messages.recover_chat", "messages.delete_chat_permanently":
		purge := a.Op == "messages.delete_chat_permanently"
		purged := map[string]bool{}
		for key, o := range f.records {
			if o.Collection != "messages.message" || o.Kind == "absent" {
				continue
			}
			var m map[string]any
			json.Unmarshal(o.Data, &m)
			raw, _ := json.Marshal(m["chat_id"])
			var reference struct {
				Ref struct {
					NativeKey string `json:"native_key"`
				} `json:"$ref"`
			}
			json.Unmarshal(raw, &reference)
			if reference.Ref.NativeKey != a.NativeKey || m["deleted_at"] == nil {
				continue
			}
			delete(m, "deleted_at")
			delete(m, "recoverable_until")
			put("message", key, m, purge)
			if purge {
				purged[key] = true
			}
		}
		if purge {
			for key, o := range f.records {
				if o.Collection != "messages.attachment" || o.Kind == "absent" {
					continue
				}
				var d map[string]any
				json.Unmarshal(o.Data, &d)
				raw, _ := json.Marshal(d["message_id"])
				var ref struct {
					Ref struct {
						NativeKey string `json:"native_key"`
					} `json:"$ref"`
				}
				json.Unmarshal(raw, &ref)
				if purged[ref.Ref.NativeKey] {
					put("attachment", key, d, true)
				}
			}
		}
		delete(d, "deleted_at")
		delete(d, "recoverable_until")
		put("chat", a.NativeKey, d, purge)
		return
	default:
		return
	}
	put("chat", a.NativeKey, d, false)
}
