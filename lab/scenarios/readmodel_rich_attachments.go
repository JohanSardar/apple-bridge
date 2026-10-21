package scenarios

import (
	"context"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func rmRecoverableMessages(ctx context.Context, w *world.World, add func(string, bool)) {
	deleted := "2026-10-01T12:00:00Z"
	add("mixed_deleted_capture", rmPut(w,
		rmObservation("messages.chat", "rich-junk-only", map[string]any{"display_name": "Bridge Test Junk", "participants": []any{}, "filter": "unknown", "is_spam": true}),
		rmObservation("messages.chat", "rich-mixed-chat", map[string]any{"display_name": "Bridge Test mixed recovery", "participants": []any{}, "filter": "known"}),
		rmObservation("messages.message", "rich-mixed-active", map[string]any{"chat_id": rmRef("messages.chat", "rich-mixed-chat"), "kind": "message", "text": "[ab-test] synthetic active", "sent_at": deleted}),
		rmObservation("messages.message", "rich-mixed-deleted", map[string]any{"chat_id": rmRef("messages.chat", "rich-mixed-chat"), "kind": "message", "text": "[ab-test] synthetic deleted", "sent_at": deleted, "deleted_at": deleted, "recoverable_until": nil}),
		rmObservation("messages.attachment", "rich-mixed-image", map[string]any{"message_id": rmRef("messages.message", "rich-mixed-deleted"), "mime_type": "image/png"})) == nil)
	aliases, _ := gatewayAliases(w)
	id := aliases[aliasKey(contracts.Messages, "messages.chat", "rich-mixed-chat")]
	for _, protocol := range []string{"rest", "mcp"} {
		client := scripted.Client{Endpoint: w.Gateway, Protocol: protocol}
		var result contracts.ChatsResult
		err := client.Call(ctx, "messages.chats", contracts.ChatsQuery{Filter: "deleted"}, &result)
		found := false
		for _, chat := range result.Chats {
			found = found || chat.Chat.ID == id
		}
		add(protocol+"_deleted_filter_contains_mixed_chat", err == nil && found)
		summary, err := rmChat(ctx, client, id)
		add(protocol+"_mixed_chat_retains_active_unread", err == nil && summary.UnreadCount == 1)
		var items contracts.CollectionResult
		err = client.Call(ctx, "messages.collection", contracts.CollectionQuery{ChatID: id, Kind: "photos"}, &items)
		add(protocol+"_recoverable_media_excluded", err == nil && len(items.Items) == 0)
		var spam contracts.ChatsResult
		err = client.Call(ctx, "messages.chats", contracts.ChatsQuery{Filter: "spam"}, &spam)
		found = false
		for _, chat := range spam.Chats {
			found = found || chat.Chat.ID == aliases[aliasKey(contracts.Messages, "messages.chat", "rich-junk-only")]
		}
		add(protocol+"_native_junk_without_messages", err == nil && found)
	}
}

func rmPlacedStickers(ctx context.Context, w *world.World, add func(string, bool)) {
	add("placed_sticker_capture", rmPut(w,
		rmObservation("messages.chat", "rm-sticker-chat", map[string]any{"display_name": "Bridge Test stickers", "participants": []any{}, "filter": "known"}),
		rmObservation("messages.message", "rm-sticker-parent", map[string]any{"kind": "message", "text": "[ab-test] synthetic sticker target", "sent_at": "2026-10-01T12:00:00Z", "chat_id": rmRef("messages.chat", "rm-sticker-chat")}),
		rmObservation("messages.message", "rm-sticker-control", map[string]any{"kind": "system"}),
		rmObservation("messages.attachment", "rm-placed-sticker", map[string]any{"message_id": rmRef("messages.message", "rm-sticker-control"), "is_sticker": true, "mime_type": "image/png", "hidden": false, "sticker": map[string]any{"target_message_id": rmRef("messages.message", "rm-sticker-parent"), "part_index": 0, "x": 0.25, "y": 0.5, "scale": 1.25, "rotation": 0.125}})) == nil)
	aliases, _ := gatewayAliases(w)
	chat := aliases[aliasKey(contracts.Messages, "messages.chat", "rm-sticker-chat")]
	target := aliases[aliasKey(contracts.Messages, "messages.message", "rm-sticker-parent")]
	for _, protocol := range []string{"rest", "mcp"} {
		var thread contracts.ThreadResult
		err := (scripted.Client{Endpoint: w.Gateway, Protocol: protocol}).Call(ctx, "messages.thread", contracts.ThreadQuery{ChatID: chat}, &thread)
		add(protocol+"_sticker_on_target_bubble", err == nil && len(thread.Items) == 1 && thread.Items[0].ID == target && len(thread.Items[0].Attachments) == 1)
		summary, err := rmChat(ctx, scripted.Client{Endpoint: w.Gateway, Protocol: protocol}, chat)
		add(protocol+"_sticker_target_preview", err == nil && summary.LastMessage != nil && summary.LastMessage.Preview.Kind == "attachments" && len(summary.LastMessage.Preview.AttachmentKinds) == 1 && summary.LastMessage.Preview.AttachmentKinds[0] == "stickers")
	}
}
