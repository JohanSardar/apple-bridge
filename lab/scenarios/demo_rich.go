package scenarios

import (
	"context"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
)

func demoRich(ctx context.Context, client scripted.Client, records []contracts.Record, coverage map[string]int, add func(string, bool)) {
	kinds, effects, styles, events, receipts := map[string]bool{}, map[string]bool{}, map[string]bool{}, map[string]bool{}, map[string]bool{}
	counts := map[string]int{}
	var chatID, stickerTarget string
	for _, record := range records {
		d := rmData(record)
		if record.Collection == "messages.chat" {
			if d["is_spam"] == true {
				counts["spam_chat"]++
			}
			if d["display_name"] == "Demo Studio" {
				chatID = record.ID
			}
			if value, ok := d["read_receipts"].(string); ok {
				receipts[value] = true
			}
			for _, key := range []string{"pinned_index", "muted_until"} {
				if d[key] != nil {
					counts[key]++
				}
			}
			if d["group_photo_attachment_id"] != nil {
				counts["chat_group_avatar"]++
			}
			if d["deleted_at"] != nil {
				counts["deleted_chat"]++
			}
		}
		if record.Collection == "messages.attachment" {
			if d["hidden"] == true {
				counts["hidden"]++
			}
			if d["genmoji"] == true {
				counts["genmoji_attachment"]++
			}
			if sticker, ok := d["sticker"].(map[string]any); ok {
				counts["sticker"]++
				stickerTarget, _ = sticker["target_message_id"].(string)
			}
			if media, ok := d["media"].(map[string]any); ok {
				if media["live"] == true {
					variants, _ := d["derivatives"].(map[string]any)
					if clip, ok := variants["live"].(map[string]any); ok && clip["digest"] != nil && clip["mime"] == "video/mp4" {
						counts["live_photo"]++
					}
				}
				if waveform, ok := media["waveform"].([]any); ok && len(waveform) == 64 {
					counts["waveform"]++
				}
			}
		}
		if record.Collection != "messages.message" {
			continue
		}
		if balloon, ok := d["balloon"].(map[string]any); ok {
			kind, _ := balloon["kind"].(string)
			kinds[kind] = true
			counts["balloon_"+kind]++
			if link, ok := balloon["link"].(map[string]any); ok && link["summary"] != nil && link["site_name"] != nil && link["icon_attachment_id"] != nil && link["video_url"] != nil {
				counts["link_metadata"]++
			}
		}
		if event, ok := d["group_event"].(map[string]any); ok {
			kind, _ := event["kind"].(string)
			events[kind] = true
			counts["group_"+kind]++
		}
		if audio, ok := d["audio"].(map[string]any); ok && audio["is_audio_message"] == true {
			counts["audio_message"]++
			if audio["kept"] == true && audio["transcription"] != nil {
				counts["audio_optional"]++
			}
		}
		for _, key := range []string{"schedule", "subject", "deleted_at"} {
			if d[key] != nil {
				counts[key]++
			}
		}
		for _, key := range []string{"has_unseen_mention", "via_satellite", "is_sos", "is_spam"} {
			if d[key] == true {
				counts[key]++
			}
		}
		if delivery, ok := d["delivery"].(map[string]any); ok {
			for _, key := range []string{"quietly", "notified_anyway"} {
				if delivery[key] == true {
					counts[key]++
				}
			}
		}
		ranges, _ := d["formatting"].([]any)
		for _, raw := range ranges {
			r, _ := raw.(map[string]any)
			if effect, ok := r["effect"].(string); ok {
				effects[effect] = true
				counts["text_effect_"+effect]++
			}
			values, _ := r["styles"].([]any)
			for _, value := range values {
				if style, ok := value.(string); ok {
					styles[style] = true
					counts["text_style_"+style]++
				}
			}
		}
		parts, _ := d["parts"].([]any)
		for _, raw := range parts {
			if p, ok := raw.(map[string]any); ok && p["kind"] == "genmoji" && p["attachment_id"] != nil && p["description"] != nil {
				counts["genmoji_part"]++
			}
		}
	}
	for key, count := range counts {
		coverage[key] = count
	}
	add("demo_paired_live_photo", counts["live_photo"] == 1)
	add("demo_every_balloon_kind", len(kinds) == 10)
	add("demo_optional_link_metadata", counts["link_metadata"] == 1)
	add("demo_synthetic_pin_and_mute", counts["pinned_index"] == 1 && counts["muted_until"] == 1)
	add("demo_all_text_styles_and_effects", len(styles) == 4 && len(effects) == 8)
	add("demo_normalized_group_events", len(events) == 8)
	add("demo_audio_message_and_waveform", counts["audio_message"] == 1 && counts["audio_optional"] == 1 && counts["waveform"] > 0)
	add("demo_sticker_genmoji_hidden", counts["sticker"] == 1 && counts["genmoji_part"] == 1 && counts["genmoji_attachment"] == 1 && counts["hidden"] == 12)
	add("demo_group_photo_and_receipt_choices", counts["chat_group_avatar"] == 1 && len(receipts) == 3)
	add("demo_recently_deleted", counts["deleted_chat"] == 1 && counts["deleted_at"] == 100)
	for _, key := range []string{"schedule", "subject", "quietly", "notified_anyway", "has_unseen_mention", "via_satellite", "is_sos", "is_spam", "spam_chat"} {
		add("demo_captured_"+key, counts[key] > 0)
	}
	for filter, want := range map[string]int{"all": 37, "known": 35, "unknown": 2, "deleted": 1, "spam": 1} {
		var result contracts.ChatsResult
		err := client.Call(ctx, "messages.chats", contracts.ChatsQuery{Filter: filter, Limit: 200}, &result)
		add("demo_filter_"+filter, err == nil && len(result.Chats) == want)
	}
	var thread contracts.ThreadResult
	err := client.Call(ctx, "messages.thread", contracts.ThreadQuery{ChatID: chatID, Anchor: stickerTarget, Limit: 1}, &thread)
	add("demo_sticker_attached_to_target_bubble", err == nil && len(thread.Items) == 1 && len(thread.Items[0].Attachments) == 1)
	for _, kind := range []string{"links", "locations"} {
		var result contracts.CollectionResult
		err := client.Call(ctx, "messages.collection", contracts.CollectionQuery{ChatID: chatID, Kind: kind}, &result)
		add("demo_collection_"+kind, err == nil && len(result.Items) == 1)
	}
}
