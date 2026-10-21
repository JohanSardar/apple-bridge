package scenarios

import (
	"fmt"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"time"
)

func webListFixture(w *world.World, update bool) (webAvatar, error) {
	avatar := webAvatar{}
	if w.Level != "sim" || w.Native {
		return avatar, fmt.Errorf("synthetic world required")
	}
	now := time.Now().UTC()
	if update {
		return avatar, rmPut(w, rmObservation("messages.message", "list-live", map[string]any{"chat_id": rmRef("messages.chat", "list-chat-001"), "sent_at": now.Add(time.Minute), "kind": "message", "text": "The plan is ready", "is_from_me": false, "parts": []any{}}))
	}
	previousTimeout := w.Control.Timeout
	w.Control.Timeout = webFixtureSetupTimeout
	defer func() { w.Control.Timeout = previousTimeout }()
	avatar, err := webAvatarFixture(w)
	if err != nil {
		return avatar, err
	}
	records := []contracts.Observation{}
	names := []string{"Maya Chen", "Weekend Crew", "Riley Parker", "Alex Morgan", "Sam Rivera"}
	for i := 0; i < 540; i++ {
		name := fmt.Sprintf("Conversation %03d", i)
		if i < len(names) {
			name = names[i]
		}
		if i == 1 {
			name = "Jordan Lee"
		}
		data := map[string]any{"address": name, "service": "iMessage"}
		if i == 4 {
			data["address"] = "photo@example.com"
		}
		records = append(records, rmObservation("messages.handle", fmt.Sprintf("list-handle-%d", i), data))
	}
	for i := 0; i < 540; i++ {
		name := fmt.Sprintf("Conversation %03d", i)
		if i < len(names) {
			name = names[i]
		}
		participants := []any{rmRef("messages.handle", fmt.Sprintf("list-handle-%d", i))}
		if i == 1 {
			participants = []any{rmRef("messages.handle", "list-handle-0"), rmRef("messages.handle", "list-handle-2"), rmRef("messages.handle", "list-handle-3"), rmRef("messages.handle", "list-handle-4")}
		}
		key := fmt.Sprintf("list-chat-%03d", i)
		style := "direct"
		if i == 1 {
			style = "group"
		}
		records = append(records, rmObservation("messages.chat", key, map[string]any{"display_name": name, "participants": participants, "style": style, "service": "iMessage", "filter": "known", "muted_until": now.Add(time.Hour), "pinned_index": nil}))
		body := "Are we still on for tomorrow?"
		if i == 2 {
			body = "See you soon"
		}
		at := now.Add(-time.Duration(i) * time.Minute)
		if i == 8 || i == 9 {
			at = now.Add(-20 * time.Minute).Truncate(time.Second).Add(time.Duration(i-8) * 100 * time.Millisecond)
		}
		if i >= 5 && i <= 7 {
			at = now.Add(-time.Duration([]int{24, 72, 240}[i-5]) * time.Hour)
		}
		messageKey := fmt.Sprintf("list-message-%03d", i)
		data := map[string]any{"chat_id": rmRef("messages.chat", key), "sent_at": at, "kind": "message", "text": body, "is_from_me": i%2 != 0, "parts": []any{}}
		records = append(records, rmObservation("messages.message", messageKey, data))
		if i == 3 {
			for j := 0; j < 2; j++ {
				records = append(records, rmObservation("messages.attachment", fmt.Sprintf("list-image-%d", j), map[string]any{"message_id": rmRef("messages.message", messageKey), "mime_type": "image/png", "filename": "image.png"}))
			}
		}
		if i == 2 {
			records = append(records, rmObservation("messages.reaction", "list-reaction", map[string]any{"message_id": rmRef("messages.message", messageKey), "sender_handle_id": rmRef("messages.handle", "list-handle-0"), "kind": "love", "part_index": 0, "sent_at": now.Add(-time.Duration(i)*time.Minute + time.Second)}))
		}
	}
	for key, chat := range map[string]string{"list-live": "list-chat-001", "list-late-incoming": "list-chat-539"} {
		records = append(records, rmObservation("messages.message", key, map[string]any{"chat_id": rmRef("messages.chat", chat), "sent_at": now.Add(-365 * 24 * time.Hour), "kind": "message", "text": "[ab-test] Earlier update", "is_from_me": true, "parts": []any{}}))
	}
	records = append(records, rmObservation("messages.chat", "list-archived", map[string]any{"display_name": "Archived fixture", "is_archived": true, "participants": []any{}, "style": "direct", "service": "iMessage"}))
	for start := 0; start < len(records); start += 100 {
		end := start + 100
		if end > len(records) {
			end = len(records)
		}
		if err := rmPut(w, records[start:end]...); err != nil {
			return avatar, fmt.Errorf("list fixture batch %d: %w", start, err)
		}
	}
	return avatar, nil
}

func webListLateIncoming(w *world.World) error {
	if w.Level != "sim" || w.Native {
		return fmt.Errorf("synthetic world required")
	}
	return rmPut(w, rmObservation("messages.message", "list-late-incoming", map[string]any{"chat_id": rmRef("messages.chat", "list-chat-539"), "sent_at": time.Now().UTC().Add(time.Minute), "kind": "message", "text": "Now at the top", "is_from_me": false, "parts": []any{}}))
}
