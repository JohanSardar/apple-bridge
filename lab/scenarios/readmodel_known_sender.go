package scenarios

import (
	"context"
	"fmt"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func rmKnownSender(ctx context.Context, w *world.World, chatID string, add func(string, bool)) {
	restricted := w.Gateway
	restricted.Token, _ = w.ScopedToken(w.Gateway, "messages:read")
	for _, known := range []bool{true, false} {
		records := []contracts.Observation{}
		participants := []any{}
		for index := 0; index < 4; index++ {
			key := fmt.Sprintf("rm-known-handle-%d", index)
			address := fmt.Sprintf("unlinked-%d@example.com", index)
			if index == 3 && known {
				address = "synthetic-0@example.invalid"
			}
			records = append(records, rmObservation("messages.handle", key, map[string]any{"address": address, "service": "iMessage"}))
			participants = append(participants, rmRef("messages.handle", key))
		}
		records = append(records, rmObservation("messages.chat", "rm-chat-000", map[string]any{"display_name": "[ab-test] Known sender", "participants": participants, "filter": "unknown", "style": "group", "service": "iMessage"}))
		add(fmt.Sprintf("known_sender_fixture_%t", known), rmPut(w, records...) == nil)
		for _, protocol := range []string{"rest", "mcp"} {
			for _, hidden := range []bool{false, true} {
				endpoint := w.Gateway
				if hidden {
					endpoint = restricted
				}
				client := scripted.Client{Endpoint: endpoint, Protocol: protocol}
				row, err := rmChat(ctx, client, chatID)
				valid := err == nil && row.KnownSender == known && len(row.AvatarHandles) == 3
				for _, handle := range row.AvatarHandles {
					valid = valid && handle.ContactID == nil
				}
				for _, filter := range []string{"known", "unknown"} {
					var result contracts.ChatsResult
					err = client.Call(ctx, "messages.chats", contracts.ChatsQuery{Filter: filter, Limit: 200}, &result)
					found := false
					for _, chat := range result.Chats {
						if chat.Chat.ID == chatID {
							found = true
						}
					}
					valid = valid && err == nil && found == (known == (filter == "known"))
				}
				add(fmt.Sprintf("%s_known_sender_%t_contacts_hidden_%t_matches_server_filters", protocol, known, hidden), valid)
			}
		}
	}
}
