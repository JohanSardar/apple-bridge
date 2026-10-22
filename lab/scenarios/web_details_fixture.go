package scenarios

import (
	"context"
	"fmt"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"time"
)

func webDetailsFixture(w *world.World) error {
	if w.Level != "sim" || w.Native {
		return fmt.Errorf("synthetic world required")
	}
	var contacts contracts.ReadResult
	if _, err := world.Request(context.Background(), w.Gateway, "/v1/op/records.query", contracts.ReadQuery{Collection: "contacts.contact", Limit: 1000}, &contacts); err != nil {
		return err
	}
	var blob any
	for _, record := range contacts.Records {
		data := rmData(record)
		if data["thumbnail_image"] != nil {
			blob = data["thumbnail_image"]
			break
		}
	}
	if blob == nil {
		return fmt.Errorf("prepared avatar blob required")
	}
	now := time.Now().UTC()
	records := []contracts.Observation{}
	for index := 0; index < 52; index++ {
		key := fmt.Sprintf("details-photo-%02d", index)
		records = append(records, rmObservation("messages.message", key, map[string]any{"chat_id": rmRef("messages.chat", "list-chat-003"), "sent_at": now.Add(time.Duration(index) * time.Second), "kind": "message", "text": "", "is_from_me": false, "parts": []any{}}), rmObservation("messages.attachment", key+"-image", map[string]any{"message_id": rmRef("messages.message", key), "mime_type": "image/png", "filename": key + ".png", "blob": blob, "media": map[string]any{"width": 64, "height": 64}}))
	}
	for _, kind := range []string{"links", "documents", "locations"} {
		key := "details-" + kind
		data := map[string]any{"chat_id": rmRef("messages.chat", "list-chat-003"), "sent_at": now.Add(time.Minute), "kind": "message", "text": "[ab-test] Shared reference", "is_from_me": false, "parts": []any{}}
		if kind == "links" {
			data["balloon"] = map[string]any{"kind": "link", "link": map[string]any{"url": "https://example.com/details", "title": "Shared reference"}}
		}
		if kind == "locations" {
			data["balloon"] = map[string]any{"kind": "find_my", "app": map[string]any{"caption": "Shared location"}}
		}
		records = append(records, rmObservation("messages.message", key, data))
		if kind == "documents" {
			records = append(records, rmObservation("messages.attachment", key+"-file", map[string]any{"message_id": rmRef("messages.message", key), "mime_type": "application/octet-stream", "filename": "reference.bin", "blob": blob}))
		}
	}
	for start := 0; start < len(records); start += 64 {
		if err := rmPut(w, records[start:min(start+64, len(records))]...); err != nil {
			return err
		}
	}
	return nil
}
