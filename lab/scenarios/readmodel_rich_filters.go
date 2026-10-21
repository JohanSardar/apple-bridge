package scenarios

import (
	"context"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func rmRichFilters(ctx context.Context, w *world.World, chatID string, add func(string, bool)) {
	at := time.Date(2026, 10, 1, 12, 0, 0, 0, time.UTC)
	records := []contracts.Observation{
		rmObservation("messages.chat", "rm-chat-000", map[string]any{"display_name": "Bridge Test known", "filter": "known", "muted_until": "2099-01-01T00:00:00Z", "pinned_index": 1}),
		rmObservation("messages.chat", "rm-chat-498", map[string]any{"display_name": "Bridge Test unknown", "filter": "unknown"}),
		rmObservation("messages.chat", "rm-chat-497", map[string]any{"display_name": "Bridge Test spam", "filter": "unknown"}),
		rmObservation("messages.chat", "rm-chat-496", map[string]any{"display_name": "Bridge Test deleted", "filter": "known", "deleted_at": at}),
	}
	for _, item := range []struct {
		key, chat, kind string
		extra           map[string]any
	}{
		{"rm-rich-unread", "rm-chat-498", "message", nil},
		{"rm-rich-spam", "rm-chat-497", "message", map[string]any{"is_spam": true}},
		{"rm-rich-deleted", "rm-chat-496", "message", map[string]any{"deleted_at": at}},
		{"rm-rich-link", "rm-chat-000", "message", map[string]any{"has_unseen_mention": true, "balloon": map[string]any{"kind": "link", "link": map[string]any{"url": "https://example.invalid/preview", "title": "Synthetic preview"}}}},
		{"rm-rich-location", "rm-chat-000", "message", map[string]any{"balloon": map[string]any{"kind": "find_my"}}},
	} {
		data := map[string]any{"kind": item.kind, "chat_id": rmRef("messages.chat", item.chat), "sent_at": at, "text": "[ab-test] synthetic rich"}
		for key, value := range item.extra {
			data[key] = value
		}
		records = append(records, rmObservation("messages.message", item.key, data))
	}
	add("rich_filter_fixtures", rmPut(w, records...) == nil)
	aliases, _ := gatewayAliases(w)
	id := func(key string) string { return aliases[aliasKey(contracts.Messages, "messages.chat", key)] }
	contains := func(rows []contracts.ChatSummary, id string) bool {
		for _, row := range rows {
			if row.Chat.ID == id {
				return true
			}
		}
		return false
	}
	for _, protocol := range []string{"rest", "mcp"} {
		c := scripted.Client{Endpoint: w.Gateway, Protocol: protocol}
		for _, filter := range []string{"all", "known", "unknown", "unread", "deleted", "spam"} {
			var out contracts.ChatsResult
			err := c.Call(ctx, "messages.chats", contracts.ChatsQuery{Filter: filter, Limit: 200}, &out)
			ok := err == nil
			switch filter {
			case "all":
				ok = ok && contains(out.Chats, id("rm-chat-498")) && !contains(out.Chats, id("rm-chat-496"))
			case "known":
				ok = ok && contains(out.Chats, chatID) && !contains(out.Chats, id("rm-chat-498"))
			case "unknown":
				ok = ok && contains(out.Chats, id("rm-chat-498")) && !contains(out.Chats, chatID)
			case "unread":
				ok = ok && contains(out.Chats, id("rm-chat-498")) && !contains(out.Chats, id("rm-chat-496"))
			case "deleted":
				ok = ok && len(out.Chats) == 1 && out.Chats[0].Chat.ID == id("rm-chat-496")
			case "spam":
				ok = ok && len(out.Chats) == 1 && out.Chats[0].Chat.ID == id("rm-chat-497")
			}
			add(protocol+"_rich_filter_"+filter, ok)
		}
		summary, err := rmChat(ctx, c, chatID)
		add(protocol+"_captured_unseen_mention", err == nil && summary.HasUnseenMention && summary.Muted != nil && *summary.Muted && summary.PinnedIndex != nil && *summary.PinnedIndex == 1)
		for _, kind := range []string{"links", "locations"} {
			var out contracts.CollectionResult
			err := c.Call(ctx, "messages.collection", contracts.CollectionQuery{ChatID: chatID, Kind: kind}, &out)
			add(protocol+"_balloon_collection_"+kind, err == nil && len(out.Items) == 1 && out.Items[0].Collection == "messages.message")
		}
	}
}
