package demo

import (
	"math/rand/v2"
	"time"
)

var phrases = []string{"The imaginary garden looks lovely today.", "Let's meet at the synthetic observatory.", "I finished the paper lantern prototype.", "Sending a little color from the demo studio.", "That sounds wonderful — see you soon!", "Could you bring the blue sketchbook?", "A quiet moment by our pretend lake.", "The example project is ready to explore."}

func messageFields(c, i, count int, rng *rand.Rand) map[string]any {
	sent := Epoch.Add(-time.Duration(730*24*60) * time.Minute).Add(time.Duration(i) * time.Duration(730*24*60/count) * time.Minute)
	if c == 0 {
		dates := []time.Time{Epoch.AddDate(-2, 0, 0), time.Date(2024, 12, 31, 23, 59, 0, 0, time.UTC), time.Date(2025, 1, 1, 0, 1, 0, 0, time.UTC), time.Date(2025, 12, 31, 23, 59, 0, 0, time.UTC), time.Date(2026, 1, 1, 0, 1, 0, 0, time.UTC), Epoch.AddDate(0, 0, -15), Epoch.AddDate(0, 0, -3), Epoch.AddDate(0, 0, -1), Epoch.Add(-6 * time.Hour), Epoch.Add(-time.Hour)}
		sent = dates[min(i/20, len(dates)-1)].Add(time.Duration(i%20) * time.Minute)
	}
	fromMe := i%3 == 0
	text := phrases[rng.IntN(len(phrases))]
	parts := []any{map[string]any{"index": 0, "kind": "text", "text": text}}
	fields := map[string]any{"chat_id": reference("messages.chat", chatKey(c)), "sender_handle_id": reference("messages.handle", handleKey((c+i%3)%60)), "is_from_me": fromMe, "sent_at": sent, "text": text, "parts": parts, "kind": "message", "decode_status": "decoded", "error_code": 0, "edit_history": []any{}, "retracted_parts": []int{}}
	sender := c
	if c == 0 || c >= 25 && c < 39 {
		sender = (c + i%(3+c%10)) % 60
	}
	if fromMe {
		sender = 0
	}
	fields["sender_handle_id"] = reference("messages.handle", handleKey(sender))
	if fromMe {
		fields["delivered_at"] = sent.Add(time.Second)
		if i%2 == 0 {
			fields["read_at"] = sent.Add(2 * time.Minute)
		}
	} else if c%4 != 0 || i < count-15 {
		fields["read_at"] = sent.Add(2 * time.Minute)
	}
	return fields
}
