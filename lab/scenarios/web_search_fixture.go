package scenarios

import (
	"fmt"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"time"
)

func webSearchFixture(w *world.World) error {
	if w.Level != "sim" || w.Native {
		return fmt.Errorf("synthetic world required")
	}
	if err := webDetailsFixture(w); err != nil {
		return err
	}
	now := time.Now().UTC()
	records := []contracts.Observation{}
	for index, key := range []string{"details-photo-00", "details-links", "details-documents", "details-locations", "search-sam"} {
		body := []string{"[ab-test] Orchard Émile <img src=x>", "[ab-test] Orchard paths", "[ab-test] Orchard document", "[ab-test] Orchard location", "[ab-test] Photos from Sam"}[index]
		chat, handle := "list-chat-003", "list-handle-3"
		if index == 4 {
			chat, handle = "list-chat-004", "list-handle-4"
		}
		data := map[string]any{"chat_id": rmRef("messages.chat", chat), "sender_handle_id": rmRef("messages.handle", handle), "sent_at": now.Add(time.Duration(index) * time.Second), "kind": "message", "text": body, "is_from_me": false, "parts": []any{}}
		if index == 1 {
			data["balloon"] = map[string]any{"kind": "link", "link": map[string]any{"url": "https://example.com/details", "title": "Orchard Garden"}}
		}
		if index == 2 {
			data["read_at"] = now
		}
		if index == 3 {
			data["balloon"] = map[string]any{"kind": "find_my", "app": map[string]any{"caption": "Orchard location"}}
		}
		records = append(records, rmObservation("messages.message", key, data))
	}
	for index := 0; index < 50; index++ {
		records = append(records, rmObservation("messages.message", fmt.Sprintf("search-page-%02d", index), map[string]any{"chat_id": rmRef("messages.chat", "list-chat-003"), "sender_handle_id": rmRef("messages.handle", "list-handle-3"), "sent_at": now.Add(time.Duration(index-100) * time.Second), "kind": "message", "text": fmt.Sprintf("[ab-test] Juniper page %02d", index), "is_from_me": false, "parts": []any{}}))
	}
	return rmPut(w, records...)
}

func webSearchUpdate(w *world.World) error {
	if w.Level != "sim" || w.Native {
		return fmt.Errorf("synthetic world required")
	}
	return rmPut(w, rmObservation("messages.message", "search-live", map[string]any{"chat_id": rmRef("messages.chat", "list-chat-003"), "sender_handle_id": rmRef("messages.handle", "list-handle-3"), "sent_at": time.Now().UTC().Add(time.Minute), "kind": "message", "text": "[ab-test] Juniper live update", "is_from_me": false, "parts": []any{}}))
}
