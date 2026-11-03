package state

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"testing"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

func TestReminderItemsCacheIsolationAndInvalidation(t *testing.T) {
	ctx := context.Background()
	store, err := Open(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	defer store.Close()
	checkpoint := ""
	put := func(collection, key string, data any) {
		t.Helper()
		raw, _ := json.Marshal(data)
		next := contracts.NewID("cache")
		_, err := store.CommitCapture(ctx, contracts.Capture{Source: contracts.Reminders, Generation: "fixture", Predecessor: checkpoint, Checkpoint: next, Observations: []contracts.Observation{{Kind: "upsert", Collection: collection, NativeKey: key, ObservedAt: time.Now().UTC(), Data: raw}}})
		if err != nil {
			t.Fatal(err)
		}
		checkpoint = next
	}
	put("reminders.list", "list", map[string]any{"title": "fixture"})
	lists, err := store.Read(ctx, trust.AllVisibility(), contracts.ReadQuery{Collection: "reminders.list"})
	if err != nil || len(lists.Records) != 1 {
		t.Fatalf("list: %v", err)
	}
	list := lists.Records[0].ID
	put("reminders.reminder", "item", map[string]any{"title": "before", "list_id": list, "completed": false})
	query := contracts.ReminderItemsQuery{List: list, TimeZone: "UTC"}
	first, err := store.ReminderItems(ctx, trust.AllVisibility(), query)
	if err != nil || first.Count != 1 {
		t.Fatalf("initial: %+v %v", first, err)
	}
	first.Sections[0].Items[0].Reminder.Data[0] = '!'
	second, err := store.ReminderItems(ctx, trust.AllVisibility(), query)
	if err != nil || !json.Valid(second.Sections[0].Items[0].Reminder.Data) {
		t.Fatalf("cached mutation leaked: %v", err)
	}
	hidden, err := store.ReminderItems(ctx, trust.Visibility{contracts.System: true}, query)
	if err != nil || hidden.Count != 0 || len(hidden.Sections) != 0 {
		t.Fatalf("scope leak: %+v %v", hidden, err)
	}
	put("reminders.reminder", "item", map[string]any{"title": "after", "list_id": list, "completed": true})
	hidden, err = store.ReminderItems(ctx, trust.AllVisibility(), query)
	if err != nil || hidden.Count != 0 || hidden.Cursor.Sequence <= second.Cursor.Sequence {
		t.Fatalf("stale completed item: %+v %v", hidden, err)
	}
	query.ShowCompleted = true
	shown, err := store.ReminderItems(ctx, trust.AllVisibility(), query)
	if err != nil || shown.Count != 1 || !bytes.Contains(shown.Sections[0].Items[0].Reminder.Data, []byte("after")) {
		t.Fatalf("query variants collided: %+v %v", shown, err)
	}
}

func TestReadModelCacheBoundsAndNestedCopies(t *testing.T) {
	cache := readModelPageCache[contracts.ThreadResult]{}
	record := contracts.Record{Data: json.RawMessage(`{"text":"fixture"}`), Coverage: contracts.Coverage{Missing: []string{"fixture"}}}
	page := contracts.ThreadResult{Items: []contracts.ThreadItem{{Record: record, Reactions: []contracts.Record{record}, Attachments: []contracts.Record{record}}}}
	cache.put("epoch/1/1", "page", page, copyThreadResult, threadResultBytes)
	page.Items[0].Attachments[0].Data[0] = '!'
	read, ok := cache.get("epoch/1/1", "page", copyThreadResult)
	if !ok || !json.Valid(read.Items[0].Attachments[0].Data) {
		t.Fatal("input mutation leaked")
	}
	read.Items[0].Reactions[0].Data[0] = '!'
	read.Items[0].Coverage.Missing[0] = "changed"
	read, ok = cache.get("epoch/1/1", "page", copyThreadResult)
	if !ok || !json.Valid(read.Items[0].Reactions[0].Data) || read.Items[0].Coverage.Missing[0] != "fixture" {
		t.Fatal("output mutation leaked")
	}
	if _, ok = cache.get("epoch/1/2", "page", copyThreadResult); ok {
		t.Fatal("stale revision returned")
	}
	for i := 0; i < 17; i++ {
		cache.put("epoch/1/1", fmt.Sprint(i), read, copyThreadResult, threadResultBytes)
	}
	if len(cache.pages) != 16 || cache.bytes > 8<<20 {
		t.Fatal("cache capacity exceeded")
	}
	oversized := contracts.ThreadResult{Items: []contracts.ThreadItem{{Record: contracts.Record{Data: make([]byte, 8<<20)}}}}
	cache.put("epoch/1/1", "oversized", oversized, copyThreadResult, threadResultBytes)
	if _, ok = cache.get("epoch/1/1", "oversized", copyThreadResult); ok {
		t.Fatal("oversized page cached")
	}
}

func TestReminderCacheNestedCopy(t *testing.T) {
	cache := readModelPageCache[contracts.ReminderItemsResult]{}
	leaf := contracts.ReminderItem{Reminder: contracts.Record{Data: json.RawMessage(`{"title":"nested"}`)}}
	page := contracts.ReminderItemsResult{Sections: []contracts.ReminderSectionItems{{Items: []contracts.ReminderItem{{Subtasks: []contracts.ReminderItem{leaf}}}}}}
	cache.put("epoch/1/1", "nested", page, copyReminderItems, reminderItemsBytes)
	read, ok := cache.get("epoch/1/1", "nested", copyReminderItems)
	if !ok {
		t.Fatal("page missing")
	}
	read.Sections[0].Items[0].Subtasks[0].Reminder.Data[0] = '!'
	read, ok = cache.get("epoch/1/1", "nested", copyReminderItems)
	if !ok || !json.Valid(read.Sections[0].Items[0].Subtasks[0].Reminder.Data) {
		t.Fatal("nested mutation leaked")
	}
}
