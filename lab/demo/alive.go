package demo

import (
	"context"
	"fmt"
	"github.com/JohanSardar/apple-bridge/lab/simapple"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"time"
)

func Alive(ctx context.Context, w *world.World, interval time.Duration) error {
	ticker := time.NewTicker(interval)
	defer ticker.Stop()
	for step := 0; ; step++ {
		select {
		case <-ctx.Done():
			return nil
		case now := <-ticker.C:
			key := fmt.Sprintf("demo-live-%08d", step)
			text := fmt.Sprintf("A new synthetic sketch arrived · %d", step+1)
			fields := map[string]any{"chat_id": reference("messages.chat", chatKey(0)), "sender_handle_id": reference("messages.handle", handleKey(1+step%2)), "is_from_me": false, "sent_at": now.UTC(), "text": text, "parts": []any{map[string]any{"index": 0, "kind": "text", "text": text}}, "kind": "message", "decode_status": "decoded", "error_code": 0}
			collection := "messages.message"
			if step%2 == 1 {
				collection = "messages.reaction"
				fields = map[string]any{"message_id": reference("messages.message", messageKey(0, 50+step%7)), "sender_handle_id": reference("messages.handle", handleKey(1+step%2)), "part_index": 0, "is_from_me": false, "kind": "emoji", "emoji": "✨", "removed": step%8 == 7, "sent_at": now.UTC()}
			}
			dataset := Dataset{Dataset: simapple.Dataset{}, Counts: map[string]int{}}
			dataset.put(collection, key, fields)
			dataset.Records[0].ObservedAt = now.UTC()
			if err := w.ControlAction("/dataset", dataset.Dataset); err != nil {
				return err
			}
		}
	}
}
