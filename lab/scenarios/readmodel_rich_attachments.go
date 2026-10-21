package scenarios

import (
	"context"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

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
