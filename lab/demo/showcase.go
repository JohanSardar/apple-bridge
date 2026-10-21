package demo

import (
	"fmt"
	"strings"
	"time"
)

func (d *Dataset) showcase(i int, key string, sent time.Time, fields map[string]any, media []mediaAsset) {
	switch i {
	case 0:
		fields["text"] = strings.Repeat("A long synthetic story about moonlit gardens and paper boats. ", 80)
	case 1:
		fields["text"] = "🌿"
	case 2:
		fields["text"] = "🌿✨"
	case 3:
		fields["text"] = "🌿✨🪁"
	case 4:
		fields["text"] = "Sketches at https://example.com/demo?view=garden and https://example.com/notes"
	case 5:
		fields["text"] = "Demo Aster00, your paper boats are ready."
		fields["parts"] = []any{map[string]any{"index": 0, "kind": "text", "text": fields["text"], "mention": "demo.person00@example.com", "utf16_location": 0, "utf16_length": 12}}
	case 7, 8, 9, 10:
		fields["reply_to_id"] = reference("messages.message", messageKey(0, i-1))
		fields["thread_originator_id"] = reference("messages.message", messageKey(0, 6))
	case 11:
		fields["text"] = "The lantern walk starts at sunset."
		fields["edited_at"] = sent.Add(3 * time.Minute)
		fields["edit_history"] = []any{map[string]any{"text": "The lantern walk starts at noon.", "timestamp": sent}, map[string]any{"text": "The lantern walk starts at dusk.", "timestamp": sent.Add(time.Minute)}}
	case 12:
		fields["text"] = ""
		fields["parts"] = []any{}
		fields["retracted_parts"] = []int{0}
		fields["retracted_at"] = sent.Add(time.Minute)
	case 13:
		fields["is_from_me"] = true
		fields["error_code"] = 1001
		fields["sender_handle_id"] = reference("messages.handle", handleKey(0))
		delete(fields, "delivered_at")
		delete(fields, "read_at")
	}
	if i >= 58 && i <= 65 {
		fromMe := i >= 62
		sender := 1
		if fromMe {
			sender = 0
		}
		fields["is_from_me"] = fromMe
		fields["sender_handle_id"] = reference("messages.handle", handleKey(sender))
		at := time.Date(2025, 1, 1, 1, 0, 0, 0, time.UTC).Add(time.Duration(i-58) * 10 * time.Second)
		fields["sent_at"] = at
		fields["read_at"] = at.Add(time.Minute)
		delete(fields, "delivered_at")
		if fromMe {
			fields["delivered_at"] = at.Add(time.Second)
		}
	}
	if i >= 14 && i < 14+len(Effects) {
		fields["effect"] = Effects[i-14]
	}
	if i >= 26 && i <= 30 {
		kind := []string{"rename", "member_added", "member_removed", "member_left", "photo_changed"}[i-26]
		fields["kind"] = "group_event"
		fields["group_event"] = map[string]any{"kind": kind, "title": "Demo Lantern Club", "item_type": []int{2, 1, 1, 3, 3}[i-26], "action_type": []int{0, 0, 1, 0, 1}[i-26], "target_handle_id": reference("messages.handle", handleKey(i)), "actor_handle_id": fields["sender_handle_id"]}
	}
	if i >= 32 && i < 32+len(media) || i >= 42 && i <= 45 {
		which := i - 32
		if i >= 42 {
			which = (i - 42) % 3
			fields["is_from_me"] = false
			fields["sender_handle_id"] = reference("messages.handle", handleKey(1))
			stackedAt := time.Date(2025, 1, 1, 0, 7, 10, 0, time.UTC).Add(time.Duration(i-42) * 10 * time.Second)
			fields["sent_at"] = stackedAt
			fields["read_at"] = stackedAt.Add(time.Minute)
			delete(fields, "delivered_at")
		}
		asset := media[which]
		attachment := fmt.Sprintf("demo-attachment-%02d", i)
		d.put("messages.attachment", attachment, map[string]any{"message_id": reference("messages.message", key), "filename": asset.name, "mime_type": asset.mime, "uti": asset.mime, "size": len(asset.data), "blob": map[string]any{"digest": d.blob(asset.data), "state": "present"}})
		fields["parts"] = []any{map[string]any{"index": 0, "kind": "attachment", "attachment_id": reference("messages.attachment", attachment)}}
		fields["text"] = ""
		if i == 32 {
			fields["parts"] = []any{map[string]any{"index": 0, "kind": "text", "text": "A synthetic portrait from our studio."}, map[string]any{"index": 1, "kind": "attachment", "attachment_id": reference("messages.attachment", attachment)}}
		}
	} else if i != 5 && i != 12 {
		fields["parts"] = []any{map[string]any{"index": 0, "kind": "text", "text": fields["text"]}}
	}
}

func (d *Dataset) preview(c int, key string, sent time.Time, fields map[string]any, media []mediaAsset) {
	switch c {
	case 2:
		asset := media[2]
		attachment := "demo-preview-image"
		d.put("messages.attachment", attachment, map[string]any{"message_id": reference("messages.message", key), "filename": asset.name, "mime_type": asset.mime, "size": len(asset.data), "blob": map[string]any{"digest": d.blob(asset.data), "state": "present"}})
		fields["text"] = ""
		fields["parts"] = []any{map[string]any{"index": 0, "kind": "attachment", "attachment_id": reference("messages.attachment", attachment)}}
	case 3:
		fields["edited_at"] = sent.Add(time.Minute)
		fields["edit_history"] = []any{map[string]any{"text": "An earlier synthetic draft.", "timestamp": sent}}
	case 4:
		fields["retracted_at"] = sent.Add(time.Minute)
		fields["retracted_parts"] = []int{0}
		fields["text"] = ""
		fields["parts"] = []any{}
	case 26:
		fields["kind"] = "group_event"
		fields["group_event"] = map[string]any{"kind": "rename", "title": "Demo Lantern Club", "item_type": 2, "action_type": 0}
	}
}
