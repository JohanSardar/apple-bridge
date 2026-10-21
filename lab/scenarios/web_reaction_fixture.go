package scenarios

import (
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/demo"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func webReactionFixture(w *world.World) (map[string]any, error) {
	message := "demo-message-00-000032"
	reaction := func(key, kind string, part int) contracts.Observation {
		data := map[string]any{"message_id": rmRef("messages.message", message), "part_index": part, "sender_handle_id": rmRef("messages.handle", "demo-handle-01"), "kind": kind, "removed": false, "is_from_me": false, "sent_at": demo.Epoch}
		if kind == "emoji" {
			data["emoji"] = "🪁"
		}
		if kind == "sticker" {
			data["sticker_id"] = "synthetic-sticker"
			data["attachment_id"] = rmRef("messages.attachment", "demo-attachment-36")
		}
		return rmObservation("messages.reaction", key, data)
	}
	if err := rmPut(w, reaction("web-reaction-part-zero", "love", 0), reaction("web-reaction-part-one", "emoji", 1), reaction("web-reaction-sticker", "sticker", 1)); err != nil {
		return nil, err
	}
	aliases, err := gatewayAliases(w)
	return map[string]any{"message": aliases[aliasKey(contracts.Messages, "messages.message", message)]}, err
}
