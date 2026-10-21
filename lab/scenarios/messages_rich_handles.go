package scenarios

import (
	"context"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func richHandleChecks(ctx context.Context, w *world.World, protocol, hid string, add func(string, bool)) {
	var handles contracts.HandlesResult
	err := (scripted.Client{Endpoint: w.Gateway, Protocol: protocol}).Call(ctx, "messages.handles", contracts.HandlesQuery{IDs: []string{hid, hid, "missing"}}, &handles)
	add(protocol+"_handles_names_initials_avatar_dedup", err == nil && len(handles.Handles) == 1 && handles.Handles[0].DisplayName == "Synthetic Sender" && handles.Handles[0].Initials == "SS" && handles.Handles[0].Avatar != nil)
	restricted := w.Gateway
	restricted.Token, _ = w.ScopedToken(w.Gateway, "messages:read")
	var private contracts.HandlesResult
	err = (scripted.Client{Endpoint: restricted, Protocol: protocol}).Call(ctx, "messages.handles", contracts.HandlesQuery{IDs: []string{hid}}, &private)
	add(protocol+"_handles_respect_contact_visibility", err == nil && len(private.Handles) == 1 && private.Handles[0].ContactID == nil && private.Handles[0].Avatar == nil && private.Handles[0].DisplayName == "reader@example.invalid")
	add(protocol+"_handles_reject_over_budget", (scripted.Client{Endpoint: w.Gateway, Protocol: protocol}).Call(ctx, "messages.handles", contracts.HandlesQuery{IDs: make([]string, 501)}, nil) != nil)
}

func richDeletedMessages(ctx context.Context, w *world.World, add func(string, bool)) {
	deleted := "2026-10-01T12:00:00Z"
	add("mixed_deleted_capture", rmPut(w,
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
	}
}
