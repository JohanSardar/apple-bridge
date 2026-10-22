package scenarios

import (
	"context"
	"fmt"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"time"
)

func rmPinPreview(ctx context.Context, w *world.World, add func(string, bool)) {
	at := time.Now().UTC()
	incoming := func(body string, read any) contracts.Observation {
		return rmObservation("messages.message", "rm-pin-incoming", map[string]any{"chat_id": rmRef("messages.chat", "rm-pin-chat"), "kind": "message", "sent_at": at, "is_from_me": false, "read_at": read, "text": body, "parts": []any{}})
	}
	records := []contracts.Observation{rmObservation("messages.chat", "rm-pin-chat", map[string]any{"display_name": "[ab-test] Pin preview", "participants": []any{}, "style": "direct", "pinned_index": 0}), incoming("[ab-test] Incoming pin message", nil)}
	for index := 0; index < 60; index++ {
		records = append(records, rmObservation("messages.message", fmt.Sprintf("rm-pin-outgoing-%02d", index), map[string]any{"chat_id": rmRef("messages.chat", "rm-pin-chat"), "kind": "message", "sent_at": at.Add(time.Duration(index+1) * time.Second), "is_from_me": true, "text": "[ab-test] Later outgoing", "parts": []any{}}))
	}
	records = append(records, rmObservation("messages.reaction", "rm-pin-removal", map[string]any{"message_id": rmRef("messages.message", "rm-pin-incoming"), "kind": "love", "removed": true, "sent_at": at.Add(61 * time.Second)}))
	add("pin_preview_60_outgoing_fixture", rmPut(w, records...) == nil)
	aliases, _ := gatewayAliases(w)
	chatID := aliases[aliasKey(contracts.Messages, "messages.chat", "rm-pin-chat")]
	messageID := aliases[aliasKey(contracts.Messages, "messages.message", "rm-pin-incoming")]
	for _, protocol := range []string{"rest", "mcp"} {
		row, err := rmChat(ctx, scripted.Client{Endpoint: w.Gateway, Protocol: protocol}, chatID)
		p := row.PinPreview
		add(protocol+"_pin_preview_latest_unread_incoming_not_outgoing_or_removal", err == nil && row.UnreadCount == 1 && p != nil && p.MessageID == messageID && p.TextPreview == "[ab-test] Incoming pin message" && p.At == at.Format(time.RFC3339Nano))
	}
	add("pin_preview_edit_capture", rmPut(w, incoming("[ab-test] Updated incoming", nil)) == nil)
	for _, protocol := range []string{"rest", "mcp"} {
		row, err := rmChat(ctx, scripted.Client{Endpoint: w.Gateway, Protocol: protocol}, chatID)
		add(protocol+"_pin_preview_updates_with_read_model", err == nil && row.PinPreview != nil && row.PinPreview.TextPreview == "[ab-test] Updated incoming")
	}
	add("pin_preview_images_capture", rmPut(w, incoming("", nil), rmObservation("messages.attachment", "rm-pin-photo-a", map[string]any{"message_id": rmRef("messages.message", "rm-pin-incoming"), "mime_type": "image/png"}), rmObservation("messages.attachment", "rm-pin-photo-b", map[string]any{"message_id": rmRef("messages.message", "rm-pin-incoming"), "mime_type": "image/jpeg"})) == nil)
	for _, protocol := range []string{"rest", "mcp"} {
		row, err := rmChat(ctx, scripted.Client{Endpoint: w.Gateway, Protocol: protocol}, chatID)
		add(protocol+"_pin_preview_attachment_wording", err == nil && row.PinPreview != nil && row.PinPreview.TextPreview == "Attachment: 2 Images")
	}
	add("pin_preview_read_capture", rmPut(w, incoming("[ab-test] Updated incoming", at.Add(2*time.Minute))) == nil)
	for _, protocol := range []string{"rest", "mcp"} {
		row, err := rmChat(ctx, scripted.Client{Endpoint: w.Gateway, Protocol: protocol}, chatID)
		add(protocol+"_read_pin_preview_is_null", err == nil && row.UnreadCount == 0 && row.PinPreview == nil)
	}
}
