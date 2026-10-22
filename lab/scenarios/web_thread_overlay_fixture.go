package scenarios

import (
	"fmt"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/demo"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func webThreadOverlayFixture(w *world.World) (map[string]any, error) {
	const root = "demo-message-00-000006"
	const distant = "web-thread-distant-reply"
	records := make([]contracts.Observation, 0, 652)
	for index := 0; index < 650; index++ {
		body := fmt.Sprintf("[ab-test transcript] bounded reply gap %d", index)
		records = append(records, rmObservation("messages.message", fmt.Sprintf("web-thread-gap-%03d", index), map[string]any{
			"chat_id": rmRef("messages.chat", "message-chat"), "sender_handle_id": rmRef("messages.handle", "demo-handle-01"), "sent_at": demo.Epoch.Add(24*time.Hour + time.Duration(index)*time.Second), "is_from_me": false, "kind": "message", "service": "iMessage", "text": body, "parts": []any{map[string]any{"index": 0, "kind": "text", "text": body}},
		}))
	}
	body := "[ab-test transcript] distant reply outside the main window"
	records = append(records, rmObservation("messages.message", distant, map[string]any{
		"chat_id": rmRef("messages.chat", "message-chat"), "sender_handle_id": rmRef("messages.handle", "demo-handle-02"), "thread_originator_id": rmRef("messages.message", root), "sent_at": demo.Epoch.Add(25 * time.Hour), "is_from_me": false, "kind": "message", "service": "iMessage", "text": body, "parts": []any{map[string]any{"index": 0, "kind": "text", "text": body}},
	}))
	records = append(records, rmObservation("messages.message", "web-media-thread-reply", map[string]any{
		"chat_id": rmRef("messages.chat", "message-chat"), "sender_handle_id": rmRef("messages.handle", "demo-handle-02"), "thread_originator_id": rmRef("messages.message", "demo-message-00-000032"), "sent_at": demo.Epoch.Add(25*time.Hour + time.Second), "is_from_me": false, "kind": "message", "service": "iMessage", "text": "[ab-test transcript] media thread reply", "parts": []any{map[string]any{"index": 0, "kind": "text", "text": "[ab-test transcript] media thread reply"}},
	}))
	previous := w.Control.Timeout
	w.Control.Timeout = 3 * time.Minute
	defer func() { w.Control.Timeout = previous }()
	for start := 0; start < len(records); start += 100 {
		if err := rmPut(w, records[start:min(start+100, len(records))]...); err != nil {
			return nil, fmt.Errorf("thread overlay batch %d: %w", start, err)
		}
	}
	aliases, err := gatewayAliases(w)
	if err != nil {
		return nil, err
	}
	result := map[string]any{"chat": aliases[aliasKey(contracts.Messages, "messages.chat", "message-chat")], "root": aliases[aliasKey(contracts.Messages, "messages.message", root)], "distant": aliases[aliasKey(contracts.Messages, "messages.message", distant)], "mediaRoot": aliases[aliasKey(contracts.Messages, "messages.message", "demo-message-00-000032")], "latest": aliases[aliasKey(contracts.Messages, "messages.message", "web-media-thread-reply")]}
	for _, value := range result {
		if value == "" {
			return nil, fmt.Errorf("thread overlay fixture incomplete")
		}
	}
	return result, nil
}
