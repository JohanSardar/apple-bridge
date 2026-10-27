package contracts

import (
	"encoding/json"
	"testing"
)

func helperMessage() map[string]any {
	return map[string]any{
		"chat_id": "cht_1", "sender_handle_id": "hnd_1", "is_from_me": true, "service": "iMessage", "sent_at": "2026-10-26T12:00:00.000Z",
		"edit_until": "2026-10-26T12:15:00.000Z", "unsend_until": "2026-10-26T12:02:00.000Z", "parts": []any{map[string]any{"index": 0, "kind": "text", "text": "hi"}},
		"text": "hi", "decode_status": "decoded", "summary_decode_status": "decoded", "summary_raw_digest": "abc", "edit_history": []any{}, "retracted_parts": []any{},
		"error_code": 0, "kind": "message", "audio": map[string]any{"is_audio_message": false}, "delivery": map[string]any{"quietly": false}, "is_spam": false,
		"has_unseen_mention": false, "via_satellite": false, "is_sos": false, "formatting": []any{}, "recoverable_until": nil,
	}
}

func check(t *testing.T, collection string, data map[string]any) []string {
	t.Helper()
	raw, err := json.Marshal(data)
	if err != nil {
		t.Fatal(err)
	}
	keys := []string{}
	for _, violation := range CheckShape(collection, raw) {
		keys = append(keys, violation.Key())
	}
	return keys
}

func TestHelperShapedMessageConforms(t *testing.T) {
	if got := check(t, "messages.message", helperMessage()); len(got) != 0 {
		t.Fatalf("violations %v", got)
	}
}

func TestShapeReportsEachKindOfDrift(t *testing.T) {
	message := helperMessage()
	message["title"] = "[ab-test] stray"
	delete(message, "service")
	message["is_from_me"] = "yes"
	message["sent_at"] = "yesterday"
	message["kind"] = nil
	got := check(t, "messages.message", message)
	want := []string{"messages.message.is_from_me: wrong_type", "messages.message.kind: unexpected_null", "messages.message.sent_at: wrong_type", "messages.message.service: missing_field", "messages.message.title: unknown_field"}
	if len(got) != len(want) {
		t.Fatalf("got %v want %v", got, want)
	}
	for i := range want {
		if got[i] != want[i] {
			t.Fatalf("got %v want %v", got, want)
		}
	}
}

func TestShapeCoversEveryMessagesCollection(t *testing.T) {
	for _, collection := range []string{"messages.message", "messages.chat", "messages.handle", "messages.reaction", "messages.attachment"} {
		if !HasShape(collection) {
			t.Fatalf("%s has no shape", collection)
		}
	}
	if HasShape("reminders.reminder") || CheckShape("reminders.reminder", json.RawMessage(`{"anything":1}`)) != nil {
		t.Fatal("non-Messages collections are not shaped here")
	}
	if got := CheckShape("messages.chat", json.RawMessage(`[1]`)); len(got) != 1 || got[0].Problem != "not_object" {
		t.Fatalf("got %v", got)
	}
	chat := map[string]any{"service": "SMS", "style": "direct", "display_name": "", "participants": []any{"hnd_1"}, "is_archived": false, "capabilities": []any{}, "filter": "known", "is_spam": false, "read_receipts": "default", "muted_until": nil, "pinned_index": nil}
	if got := check(t, "messages.chat", chat); len(got) != 0 {
		t.Fatalf("chat violations %v", got)
	}
}
