package scenarios

import (
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/demo"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"time"
)

func webReactionFixture(w *world.World, removed bool) (map[string]any, error) {
	message := "demo-message-00-000032"
	reaction := func(key, kind string, part int) contracts.Observation {
		handle := "demo-handle-01"
		if kind == "like" {
			handle = "demo-handle-02"
		}
		if kind == "laugh" {
			handle = "demo-handle-03"
		}
		if kind == "question" {
			handle = "demo-handle-04"
		}
		offset := map[string]time.Duration{"question": -10, "emoji": 10, "sticker": 20, "laugh": 30, "like": 40}[kind]
		data := map[string]any{"message_id": rmRef("messages.message", message), "part_index": part, "sender_handle_id": rmRef("messages.handle", handle), "kind": kind, "removed": removed, "is_from_me": false, "sent_at": demo.Epoch.Add(offset * time.Second)}
		if kind == "emoji" {
			data["emoji"] = "🪁"
		}
		if kind == "sticker" {
			data["sticker_id"] = "synthetic-sticker"
			data["attachment_id"] = rmRef("messages.attachment", "demo-attachment-36")
		}
		return rmObservation("messages.reaction", key, data)
	}
	if err := rmPut(w, reaction("web-reaction-part-zero", "love", 0), reaction("web-reaction-part-one", "emoji", 1), reaction("web-reaction-sticker", "sticker", 1), reaction("web-reaction-a-like", "like", 1), reaction("web-reaction-b-laugh", "laugh", 1), reaction("web-reaction-c-question", "question", 1)); err != nil {
		return nil, err
	}
	aliases, err := gatewayAliases(w)
	return map[string]any{"message": aliases[aliasKey(contracts.Messages, "messages.message", message)]}, err
}
