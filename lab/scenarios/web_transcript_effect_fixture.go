package scenarios

import (
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func webTranscriptEffectFixture(w *world.World, effect string, sent bool) (map[string]any, error) {
	key := contracts.NewID("web-effect")
	body := "[ab-test effects] Orbit https://example.com/notes"
	handle := "demo-handle-01"
	if sent {
		handle = "demo-handle-00"
	}
	data := map[string]any{"chat_id": rmRef("messages.chat", "message-chat"), "sender_handle_id": rmRef("messages.handle", handle), "sent_at": time.Now().UTC(), "is_from_me": sent, "kind": "message", "service": "iMessage", "text": body, "effect": effect, "parts": []any{map[string]any{"index": 0, "kind": "text", "text": body}}, "formatting": []any{map[string]any{"utf16_location": strings.Index(body, "Orbit"), "utf16_length": 5, "styles": []string{"bold", "italic"}, "effect": "shake"}}}
	if err := rmPut(w, rmObservation("messages.message", key, data)); err != nil {
		return nil, err
	}
	aliases, err := gatewayAliases(w)
	return map[string]any{"message": aliases[aliasKey(contracts.Messages, "messages.message", key)], "text": body}, err
}
