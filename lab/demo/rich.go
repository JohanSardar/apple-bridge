package demo

import (
	"fmt"
	"time"
)

func (d *Dataset) richShowcase(i int, key string, sent time.Time, fields map[string]any, media []mediaAsset) {
	if i >= 100 && i < 110 {
		kind := []string{"link", "app", "digital_touch", "handwriting", "poll", "check_in", "apple_cash", "find_my", "chatbot", "unknown"}[i-100]
		attachment := fmt.Sprintf("demo-balloon-image-%d", i)
		d.richAttachment(attachment, key, media[1], map[string]any{"hidden": true})
		balloon := map[string]any{"bundle_id": "synthetic." + kind, "kind": kind, "decode_status": "decoded", "app_name": "Demo " + kind}
		if kind == "link" {
			balloon["bundle_id"] = "com.apple.messages.URLBalloonProvider"
			balloon["link"] = map[string]any{"url": "https://example.com/garden", "title": "The imaginary garden", "summary": "A generated preview from the synthetic studio.", "site_name": "Example Garden", "image_attachment_id": reference("messages.attachment", attachment), "icon_attachment_id": reference("messages.attachment", attachment), "video_url": "https://example.com/garden/preview.mp4"}
		} else {
			balloon["app"] = map[string]any{"caption": "Demo " + kind, "subcaption": "Synthetic app preview", "trailing_caption": "Open in the demo", "image_attachment_id": reference("messages.attachment", attachment), "url": "https://example.com/apps/" + kind}
		}
		fields["text"], fields["parts"], fields["balloon"] = "", []any{}, balloon
	}
	switch i {
	case 39:
		fields["audio"] = map[string]any{"is_audio_message": true, "played_at": sent.Add(time.Minute), "expirable": true, "kept": true, "transcription": "A synthetic hello from the demo studio."}
	case 110:
		target := messageKey(0, 111)
		d.richAttachment("demo-placed-sticker", key, media[4], map[string]any{"hidden": true, "is_sticker": true, "sticker": map[string]any{"target_message_id": reference("messages.message", target), "part_index": 0, "x": 0.65, "y": 0.3, "scale": 1.1, "rotation": -0.12}})
	case 111:
		fields["text"] = "A little color for the paper boats."
		fields["parts"] = []any{map[string]any{"index": 0, "kind": "text", "text": fields["text"]}}
	case 112:
		d.richAttachment("demo-genmoji", key, media[4], map[string]any{"genmoji": true, "description": "A synthetic glowing paper kite"})
		fields["text"] = ""
		fields["parts"] = []any{map[string]any{"index": 0, "kind": "genmoji", "attachment_id": reference("messages.attachment", "demo-genmoji"), "description": "A synthetic glowing paper kite"}}
	case 113:
		fields["text"] = "Bold paper boats"
		fields["parts"] = []any{map[string]any{"index": 0, "kind": "text", "text": fields["text"]}}
		fields["formatting"] = []any{map[string]any{"utf16_location": 0, "utf16_length": 4, "styles": []string{"bold", "italic"}}, map[string]any{"utf16_location": 5, "utf16_length": 5, "styles": []string{"underline", "strikethrough"}}}
	case 114, 115, 116, 117, 118, 119, 120, 121:
		fields["text"] = "Moving paper boats"
		fields["parts"] = []any{map[string]any{"index": 0, "kind": "text", "text": fields["text"]}}
		fields["formatting"] = []any{map[string]any{"utf16_location": 0, "utf16_length": 6, "styles": []string{}, "effect": []string{"big", "small", "shake", "nod", "explode", "ripple", "bloom", "jitter"}[i-114]}}
	case 122:
		fields["is_from_me"] = true
		fields["schedule"] = map[string]any{"state": "native_3", "scheduled_for": Epoch.Add(time.Hour)}
		delete(fields, "delivered_at")
		delete(fields, "read_at")
	case 123:
		fields["subject"] = "Synthetic studio update"
		fields["delivery"] = map[string]any{"quietly": true, "notified_anyway": false}
	case 124:
		fields["delivery"] = map[string]any{"quietly": true, "notified_anyway": true}
		fields["has_unseen_mention"] = true
	case 125:
		fields["via_satellite"], fields["is_sos"] = true, true
	case 129:
		d.richAttachment("demo-live-still", key, media[2], nil)
		movie := media[6]
		movie.name, movie.mime = "demo-live.mov", "video/quicktime"
		d.richAttachment("demo-live-movie", key, movie, map[string]any{"hidden": true})
		d.LivePhotos["demo-live-still"] = "demo-live-movie"
		fields["text"] = ""
		fields["parts"] = []any{map[string]any{"index": 0, "kind": "attachment", "attachment_id": reference("messages.attachment", "demo-live-still")}}
	case 126, 127, 128:
		fields["kind"], fields["text"], fields["parts"] = "group_event", "", []any{}
		fields["group_event"] = map[string]any{"kind": []string{"phone_changed", "photo_removed", "unknown"}[i-126], "item_type": []int{1, 3, 99}[i-126], "action_type": []int{0, 2, 0}[i-126], "title": "", "actor_handle_id": fields["sender_handle_id"]}
		if i == 126 {
			fields["group_event"].(map[string]any)["target_handle_id"] = fields["sender_handle_id"]
		}
	}
}

func (d *Dataset) richAttachment(id, message string, asset mediaAsset, extra map[string]any) {
	fields := map[string]any{"message_id": reference("messages.message", message), "filename": asset.name, "mime_type": asset.mime, "uti": asset.mime, "size": len(asset.data), "blob": map[string]any{"digest": d.blob(asset.data), "state": "present"}}
	for key, value := range extra {
		fields[key] = value
	}
	d.put("messages.attachment", id, fields)
}
