package scenarios

import (
	"context"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/demo"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func checkThreadOriginatorLive(ctx context.Context, w *world.World, fixture threadFixture, add func(string, bool)) {
	observer, err := watch(ctx, w.Gateway, false, func(contracts.Record) bool { return true })
	add("thread_rest_observer_registered", err == nil)
	if err != nil {
		return
	}
	defer observer.close()
	type window struct {
		Items        []contracts.ThreadItem
		Older, Newer int
	}
	query := func() (window, error) {
		var result contracts.ThreadResult
		err := (scripted.Client{Endpoint: w.Gateway}).Call(ctx, "messages.thread", map[string]any{"chat_id": fixture.chat, "thread_originator_id": fixture.root, "anchor": "latest", "limit": 200}, &result)
		view := window{Items: result.Items}
		if err != nil {
			return view, err
		}
		if result.Before != "" {
			var older contracts.ThreadResult
			err = (scripted.Client{Endpoint: w.Gateway}).Call(ctx, "messages.thread", map[string]any{"chat_id": fixture.chat, "thread_originator_id": fixture.root, "before": result.Before, "limit": 200}, &older)
			view.Older = len(older.Items)
		}
		if result.After != "" {
			view.Newer = 1
		}
		return view, err
	}
	snapshot, err := query()
	add("filtered_snapshot_scoped_counts", err == nil && len(snapshot.Items) == 200 && snapshot.Older == 6 && snapshot.Newer == 0)
	put := func(key, body string, originator bool) error {
		data := map[string]any{"chat_id": rmRef("messages.chat", "demo-chat-39"), "sent_at": demo.Epoch.Add(time.Hour), "text": body, "kind": "message", "parts": []any{map[string]any{"index": 0, "kind": "text", "text": body}}}
		if originator {
			data["thread_originator_id"] = rmRef("messages.message", "transcript-originator-root")
		}
		return rmPut(w, rmObservation("messages.message", key, data))
	}
	await := func(text string) bool {
		return w.Await(5*time.Second, func() bool {
			observer.mu.Lock()
			defer observer.mu.Unlock()
			for _, seen := range observer.seen {
				if seen.Change.Record.Collection == "messages.message" && rmData(seen.Change.Record)["text"] == text {
					return true
				}
			}
			return false
		}) == nil
	}
	err = put("transcript-originator-unrelated", "An unrelated synthetic message", false)
	observed := err == nil && await("An unrelated synthetic message")
	unrelated, readErr := query()
	add("unrelated_live_message_excluded", observed && readErr == nil && len(unrelated.Items) == 200 && unrelated.Older == 6 && unrelated.Newer == 0)
	err = put("transcript-originator-live", "transcriptoriginatorlivefixture", true)
	observed = err == nil && await("transcriptoriginatorlivefixture")
	inserted, readErr := query()
	valid := observed && readErr == nil && len(inserted.Items) == 200 && inserted.Older == 7 && inserted.Newer == 0
	found := false
	for _, item := range inserted.Items {
		found = found || rmData(item.Record)["text"] == "transcriptoriginatorlivefixture" && rmData(item.Record)["thread_originator_id"] == fixture.root
	}
	add("distant_live_reply_inserted_with_filtered_counts", valid && found)
	err = rmPut(w, rmObservation("messages.message", "transcript-originator-reply-204", map[string]any{"chat_id": rmRef("messages.chat", "demo-chat-39"), "sent_at": demo.Epoch.Add(204 * time.Second), "kind": "message", "text": "[ab-test originator] reassigned"}))
	observed = err == nil && await("[ab-test originator] reassigned")
	removed, readErr := query()
	valid = observed && readErr == nil && len(removed.Items) == 200 && removed.Older == 6 && removed.Newer == 0
	for _, item := range removed.Items {
		valid = valid && item.ID != fixture.replies[204]
	}
	add("reassigned_live_reply_removed", valid)
}
