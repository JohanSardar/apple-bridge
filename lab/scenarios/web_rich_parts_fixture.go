package scenarios

import (
	"fmt"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/demo"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func webRichPartsFixture(w *world.World) (map[string]any, error) {
	records := make([]contracts.Observation, 0, 5)
	keys := []string{"sentText", "sentCard", "receivedText", "receivedCard"}
	for index, key := range keys {
		sent := index < 2
		handle := "demo-handle-01"
		if sent {
			handle = "demo-handle-00"
		}
		body := "[ab-test transcript] A captured card follows this text."
		fields := map[string]any{"chat_id": rmRef("messages.chat", "message-chat"), "sender_handle_id": rmRef("messages.handle", handle), "sent_at": demo.Epoch.Add(72*time.Hour + time.Duration(index/2)*3*time.Hour + time.Duration(index%2)*10*time.Second), "is_from_me": sent, "kind": "message", "service": "iMessage", "text": body, "parts": []any{map[string]any{"index": 0, "kind": "text", "text": body}}}
		if index%2 == 1 {
			fields["text"] = ""
			fields["parts"] = []any{}
			fields["balloon"] = map[string]any{"kind": "link", "link": map[string]any{"url": "https://example.com/bridge-demo", "title": "Synthetic shared card"}}
		}
		records = append(records, rmObservation("messages.message", "web-rich-"+key, fields))
	}
	keys = append(keys, "missingGenmoji")
	records = append(records, rmObservation("messages.message", "web-rich-missingGenmoji", map[string]any{"chat_id": rmRef("messages.chat", "message-chat"), "sender_handle_id": rmRef("messages.handle", "demo-handle-01"), "sent_at": demo.Epoch.Add(84 * time.Hour), "is_from_me": false, "kind": "message", "text": "", "parts": []any{map[string]any{"index": 0, "kind": "genmoji", "description": "Unavailable synthetic Genmoji"}}}))

	previous := w.Control.Timeout
	w.Control.Timeout = 3 * time.Minute
	defer func() { w.Control.Timeout = previous }()
	if err := rmPut(w, records...); err != nil {
		return nil, err
	}
	aliases, err := gatewayAliases(w)
	if err != nil {
		return nil, err
	}
	result := map[string]any{}
	for _, key := range keys {
		id := aliases[aliasKey(contracts.Messages, "messages.message", "web-rich-"+key)]
		if id == "" {
			return nil, fmt.Errorf("rich parts fixture incomplete")
		}
		result[key] = id
	}
	return result, nil
}
