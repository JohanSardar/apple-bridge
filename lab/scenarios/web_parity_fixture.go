package scenarios

import (
	"encoding/json"
	"fmt"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"time"
)

func webParityFixture(w *world.World) (map[string]string, error) {
	if w.Level != "sim" || w.Native {
		return nil, fmt.Errorf("synthetic world required")
	}
	records, err := w.Inspect()
	if err != nil {
		return nil, err
	}
	for _, record := range records {
		if record.Collection != "messages.chat" {
			continue
		}
		if record.NativeKey != "list-chat-000" && record.NativeKey != "list-chat-001" && record.NativeKey != "list-chat-002" {
			continue
		}
		var data map[string]any
		if err := json.Unmarshal(record.Data, &data); err != nil {
			return nil, err
		}
		data["muted"], data["read_receipts"] = false, "default"
		if err := w.ControlAction("/external-change", map[string]any{"key": record.NativeKey, "collection": record.Collection, "data": data}); err != nil {
			return nil, err
		}
	}
	if err := rmPut(w, rmObservation("messages.chat", "parity-deleted", map[string]any{"display_name": "[ab-test] Deleted conversation", "participants": []any{}, "style": "direct", "service": "iMessage", "deleted_at": time.Now().UTC(), "recoverable_until": nil}), rmObservation("messages.message", "parity-deleted-message", map[string]any{"chat_id": rmRef("messages.chat", "parity-deleted"), "text": "[ab-test] Deleted fixture", "is_from_me": true, "sent_at": time.Now().UTC(), "deleted_at": time.Now().UTC(), "parts": []any{}})); err != nil {
		return nil, err
	}
	if err := w.FlushCaptures(30 * time.Second); err != nil {
		return nil, err
	}
	aliases, err := gatewayAliases(w)
	if err != nil {
		return nil, err
	}
	result := map[string]string{}
	for _, key := range []string{"list-chat-000", "list-chat-001", "list-chat-002", "parity-deleted"} {
		result[key] = aliases[aliasKey(contracts.Messages, "messages.chat", key)]
	}
	return result, nil
}
