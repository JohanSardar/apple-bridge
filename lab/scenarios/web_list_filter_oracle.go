package scenarios

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"syscall"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func webListFilterOracle(ctx context.Context, w *world.World, hidden bool) (map[string][]contracts.ChatSummary, error) {
	endpoint := w.Gateway
	if hidden {
		token, err := w.ScopedToken(endpoint, "messages:read")
		if err != nil {
			return nil, err
		}
		endpoint.Token = token
	}
	client := scripted.Client{Endpoint: endpoint, Protocol: "rest"}
	result := map[string][]contracts.ChatSummary{}
	for _, filter := range []string{"all", "known", "unknown", "unread", "deleted", "spam"} {
		result[filter] = []contracts.ChatSummary{}
		after := ""
		for {
			var page contracts.ChatsResult
			if err := client.Call(ctx, "messages.chats", contracts.ChatsQuery{Filter: filter, Limit: 200, After: after}, &page); err != nil {
				return nil, err
			}
			result[filter] = append(result[filter], page.Chats...)
			if page.Next == "" {
				break
			}
			after = page.Next
		}
	}
	return result, nil
}

func webListFilterGroup(w *world.World, known, unread bool) error {
	if w.Level != "sim" || w.Native {
		return fmt.Errorf("synthetic world required")
	}
	now := time.Now().UTC()
	records := []contracts.Observation{}
	participants := []any{}
	for index, key := range []string{"first", "second", "third", "fourth"} {
		data := map[string]any{"address": key + "@example.com", "service": "iMessage"}
		if index == 3 && known {
			data["address"] = "photo@example.com"
		}
		records = append(records, rmObservation("messages.handle", "list-filter-handle-"+key, data))
		participants = append(participants, rmRef("messages.handle", "list-filter-handle-"+key))
	}
	message := map[string]any{"chat_id": rmRef("messages.chat", "list-filter-group"), "sent_at": now.Add(2 * time.Minute), "kind": "message", "text": "[ab-test] Fourth participant fixture", "is_from_me": false, "read_at": nil, "parts": []any{}}
	if !unread {
		message["read_at"] = now
	}
	records = append(records, rmObservation("messages.chat", "list-filter-group", map[string]any{"display_name": "Fourth linked participant", "participants": participants, "filter": "unknown", "style": "group", "service": "iMessage", "is_archived": false}), rmObservation("messages.message", "list-filter-group-message", message))
	return rmPut(w, records...)
}

func webListCacheDelta(w *world.World, index int) error {
	if w.Level != "sim" || w.Native {
		return fmt.Errorf("synthetic world required")
	}
	if index < 0 || index >= 50 {
		return fmt.Errorf("invalid synthetic delta")
	}
	return rmPut(w, rmObservation("messages.message", "list-cache-burst", map[string]any{"chat_id": rmRef("messages.chat", "list-chat-000"), "sent_at": time.Now().UTC().Add(10 * time.Minute), "kind": "message", "text": fmt.Sprintf("[ab-test] Cache burst %d", index), "is_from_me": false, "parts": []any{}}))
}

func webListHideContacts(w *world.World, deviceID string) error {
	if w.Level != "sim" || w.Native {
		return fmt.Errorf("synthetic world required")
	}
	path := filepath.Join(w.Gateway.Home, "credentials.json")
	lock, err := os.OpenFile(filepath.Join(w.Gateway.Home, "credentials.lock"), os.O_CREATE|os.O_RDWR, 0600)
	if err != nil {
		return err
	}
	defer lock.Close()
	if err = syscall.Flock(int(lock.Fd()), syscall.LOCK_EX); err != nil {
		return err
	}
	defer syscall.Flock(int(lock.Fd()), syscall.LOCK_UN)
	raw, err := os.ReadFile(path)
	if err != nil {
		return err
	}
	var fixture map[string]json.RawMessage
	if err = json.Unmarshal(raw, &fixture); err != nil {
		return err
	}
	var principals []map[string]json.RawMessage
	if err = json.Unmarshal(fixture["principals"], &principals); err != nil {
		return err
	}
	found := false
	for _, principal := range principals {
		var id, kind string
		if json.Unmarshal(principal["id"], &id) != nil || json.Unmarshal(principal["kind"], &kind) != nil {
			return contracts.ErrCorrupt
		}
		if id != deviceID || kind != "device" {
			continue
		}
		var revision uint64
		if err = json.Unmarshal(principal["revision"], &revision); err != nil {
			return err
		}
		principal["grants"], _ = json.Marshal([]contracts.Grant{"messages:read"})
		principal["revision"], _ = json.Marshal(revision + 1)
		found = true
	}
	if !found {
		return contracts.ErrNotFound
	}
	fixture["principals"], _ = json.Marshal(principals)
	raw, err = json.Marshal(fixture)
	if err != nil {
		return err
	}
	if err = os.WriteFile(path+".list-scoped", raw, 0600); err != nil {
		return err
	}
	return os.Rename(path+".list-scoped", path)
}
