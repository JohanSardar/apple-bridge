package scenarios

import (
	"context"
	"fmt"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"strings"
	"time"
)

func webListFilterFixture(w *world.World) error {
	if w.Level != "sim" || w.Native {
		return fmt.Errorf("synthetic world required")
	}
	now := time.Now().UTC()
	records := []contracts.Observation{}
	for _, item := range []struct {
		key, title, filter string
		deleted, spam      bool
	}{{"unknown", "Unknown sender fixture", "unknown", false, false}, {"deleted", "Deleted fixture", "known", true, false}, {"spam", "Spam fixture", "unknown", false, true}} {
		key := "list-filter-" + item.key
		data := map[string]any{"display_name": item.title, "filter": item.filter, "participants": []any{rmRef("messages.handle", "list-handle-20")}, "style": "direct", "service": "iMessage", "is_archived": false}
		if item.deleted {
			data["deleted_at"] = now.Add(-24 * time.Hour)
			data["recoverable_until"] = now.Add(29 * 24 * time.Hour)
		}
		records = append(records, rmObservation("messages.chat", key, data), rmObservation("messages.message", key+"-message", map[string]any{"chat_id": rmRef("messages.chat", key), "sent_at": now.Add(time.Minute), "kind": "message", "text": "[ab-test] Filter fixture", "is_from_me": false, "is_spam": item.spam, "parts": []any{}}))
	}
	return rmPut(w, records...)
}

func webListEmptyFixture(ctx context.Context, w *world.World) error {
	if w.Level != "sim" || w.Native {
		return fmt.Errorf("synthetic world required")
	}
	records := []contracts.Observation{}
	for _, collection := range []string{"messages.chat", "messages.message"} {
		after := ""
		for {
			var page contracts.ReadResult
			_, err := world.Request(ctx, w.Gateway, "/v1/op/records.query", contracts.ReadQuery{Collection: collection, IncludeAliases: true, Limit: 1000, After: after}, &page)
			if err != nil {
				return err
			}
			native := map[string]string{}
			for _, alias := range page.Aliases {
				if strings.HasPrefix(alias.NativeKey, "list-") || strings.HasPrefix(alias.NativeKey, "demo-") || strings.HasPrefix(alias.NativeKey, "message-") {
					native[alias.RecordID] = alias.NativeKey
				}
			}
			for _, record := range page.Records {
				data := rmData(record)
				if collection == "messages.message" && data["deleted_at"] == nil {
					continue
				}
				key := native[record.ID]
				if key == "" {
					return fmt.Errorf("unowned synthetic record")
				}
				if collection == "messages.chat" {
					data["is_archived"], data["pinned_index"] = true, nil
				}
				data["deleted_at"], data["recoverable_until"] = nil, nil
				records = append(records, rmObservation(collection, key, data))
			}
			if page.Next == "" {
				break
			}
			after = page.Next
		}
	}
	for start := 0; start < len(records); start += 100 {
		if err := rmPut(w, records[start:min(start+100, len(records))]...); err != nil {
			return err
		}
	}
	return nil
}
