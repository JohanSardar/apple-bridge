package scenarios

import (
	"context"
	"os/exec"
	"reflect"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func runRichMessages(ctx context.Context, s Scenario, level string) Evidence {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e := Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, GitSHA: strings.TrimSpace(string(sha)), At: time.Now().UTC(), Assertions: []Assertion{}, Latency: map[string]Latency{}, Metrics: map[string]any{}}
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	if level == "real" {
		return runRealRichMessages(ctx, s, e)
	}
	if s.ID == "MSG-DECODE-BALLOON" || s.ID == "MSG-DECODE-FORMATTING" {
		err := exec.CommandContext(ctx, "packaging/dev/check-decoders.sh").Run()
		add("scrubbed_native_decoder_fixtures", err == nil)
		if err != nil {
			return e
		}
	}
	if s.ID == "MSG-READ-RICH-FIELDS" {
		command := exec.CommandContext(ctx, "packaging/dev/check-messages-capture.sh", "--rich")
		add("native_rich_capture_and_live_mutation", command.Run() == nil)
		add("independent_truth_synthetic_field_counts", exec.CommandContext(ctx, "packaging/dev/check-messages-truth.sh").Run() == nil)
	}
	w, err := world.New(ctx, "sim", 0, 0, world.BlobOptions{Scope: "messages:read,contacts:read,system:read"})
	if err != nil {
		add("world_start", false)
		return e
	}
	defer w.Close()
	at := time.Date(2026, 10, 1, 12, 0, 0, 0, time.UTC)
	observations := []contracts.Observation{}
	for _, filter := range []string{"known", "unknown", "deleted", "spam"} {
		d := map[string]any{"display_name": "Bridge Test " + filter, "participants": []any{}, "filter": filter, "read_receipts": "default", "muted_until": nil, "pinned_index": nil}
		if filter == "spam" {
			d["filter"] = "unknown"
		}
		if filter == "deleted" {
			d["filter"] = "known"
			d["deleted_at"] = at
			d["recoverable_until"] = at.Add(30 * 24 * time.Hour)
		}
		observations = append(observations, rmObservation("messages.chat", "rich-"+filter, d))
	}
	for i, kind := range []string{"link", "app", "digital_touch", "handwriting", "poll", "check_in", "apple_cash", "find_my", "chatbot", "unknown"} {
		d := map[string]any{"title": "[ab-test] rich-" + kind, "text": "[ab-test] rich-" + kind, "kind": "message", "chat_id": rmRef("messages.chat", "rich-known"), "sent_at": at.Add(time.Duration(i) * time.Second), "balloon": map[string]any{"bundle_id": "synthetic." + kind, "kind": kind}, "has_unseen_mention": true}
		if kind == "link" {
			d["subject"] = "Synthetic subject"
			d["audio"] = map[string]any{"is_audio_message": true, "played_at": at, "expirable": true, "kept": nil, "transcription": "Synthetic transcript"}
			d["schedule"] = map[string]any{"state": "native_3", "scheduled_for": at}
			d["delivery"] = map[string]any{"quietly": true, "notified_anyway": true}
			d["via_satellite"] = true
			d["is_sos"] = true
			d["formatting"] = []any{map[string]any{"utf16_location": 0, "utf16_length": 2, "styles": []string{"bold", "italic"}, "effect": "ripple"}}
			d["parts"] = []any{map[string]any{"index": 0, "kind": "genmoji", "attachment_id": rmRef("messages.attachment", "rich-glyph"), "description": "Synthetic glyph"}}
			d["balloon"] = map[string]any{"bundle_id": "com.apple.messages.URLBalloonProvider", "kind": "link", "link": map[string]any{"url": "https://example.invalid/preview", "title": "Synthetic link", "image_attachment_id": rmRef("messages.attachment", "rich-image")}}
		}
		key := "rich-" + kind
		if kind == "unknown" {
			key = "rich-balloon-unknown"
		}
		observations = append(observations, rmObservation("messages.message", key, d))
	}
	observations = append(observations, rmObservation("messages.message", "rich-spam-message", map[string]any{"title": "[ab-test] spam", "kind": "message", "is_spam": true, "chat_id": rmRef("messages.chat", "rich-spam"), "sent_at": at}))
	observations = append(observations,
		rmObservation("messages.message", "rich-sticker-control", map[string]any{"kind": "system", "text": "[ab-test] synthetic sticker control", "sent_at": at}),
		rmObservation("messages.attachment", "rich-glyph", map[string]any{"message_id": rmRef("messages.message", "rich-sticker-control"), "filename": "synthetic.png", "mime_type": "image/png", "is_sticker": true, "hidden": true, "genmoji": true, "sticker": map[string]any{"target_message_id": rmRef("messages.message", "rich-link"), "part_index": 0, "x": 0.25, "y": 0.5, "scale": 1.25, "rotation": 0.125}}),
		rmObservation("messages.attachment", "rich-image", map[string]any{"message_id": rmRef("messages.message", "rich-link"), "filename": "synthetic-preview.png", "mime_type": "image/png", "hidden": true}),
		rmObservation("messages.handle", "rich-handle", map[string]any{"address": "reader@example.invalid", "service": "iMessage"}))
	add("contact_capture", w.ControlAction("/external-change", map[string]any{"key": "rich-contact", "collection": "contacts.contact", "data": map[string]any{"given_name": "Synthetic", "family_name": "Sender", "emails": []any{map[string]any{"value": "reader@example.invalid"}}, "thumbnail_image": map[string]any{"digest": strings.Repeat("a", 64), "state": "present"}}}) == nil)
	add("contact_committed", w.FlushCaptures(20*time.Second) == nil)
	add("rich_capture", rmPut(w, observations...) == nil)
	aliases, _ := gatewayAliases(w)
	chat := aliases[aliasKey(contracts.Messages, "messages.chat", "rich-known")]
	for _, protocol := range []string{"rest", "mcp"} {
		c := scripted.Client{Endpoint: w.Gateway, Protocol: protocol}
		for _, filter := range []string{"known", "unknown", "unread", "deleted", "spam"} {
			var result contracts.ChatsResult
			err = c.Call(ctx, "messages.chats", contracts.ChatsQuery{Filter: filter}, &result)
			want := 1
			if filter == "unknown" {
				want = 2
			}
			if filter == "unread" {
				want = 2
			}
			e.Metrics[protocol+"_filter_"+filter] = len(result.Chats)
			add(protocol+"_filter_"+filter, err == nil && len(result.Chats) == want)
		}
		summary, err := rmChat(ctx, c, chat)
		add(protocol+"_unseen_mention", err == nil && summary.HasUnseenMention)
		for _, kind := range []string{"links", "locations"} {
			var result contracts.CollectionResult
			err = c.Call(ctx, "messages.collection", contracts.CollectionQuery{ChatID: chat, Kind: kind}, &result)
			add(protocol+"_collection_"+kind, err == nil && len(result.Items) == 1)
		}
		var thread contracts.ThreadResult
		err = c.Call(ctx, "messages.thread", contracts.ThreadQuery{ChatID: chat}, &thread)
		add(protocol+"_every_balloon_kind_passes_through", err == nil && len(thread.Items) == 10)
		for _, item := range thread.Items {
			d := rmData(item.Record)
			if d["subject"] == "Synthetic subject" {
				e.Metrics[protocol+"_rich_attachment_count"] = len(item.Attachments)
				add(protocol+"_rich_fields_and_attachment_refs", d["audio"] != nil && d["schedule"] != nil && d["delivery"] != nil && d["formatting"] != nil && d["parts"] != nil && d["via_satellite"] == true && d["is_sos"] == true && len(item.Attachments) == 2)
			}
		}
		hid := aliases[aliasKey(contracts.Messages, "messages.handle", "rich-handle")]
		richHandleChecks(ctx, w, protocol, hid, add)
	}
	target := aliases[aliasKey(contracts.Messages, "messages.message", "rich-link")]
	var before contracts.ThreadResult
	c := scripted.Client{Endpoint: w.Gateway}
	c.Call(ctx, "messages.thread", contracts.ThreadQuery{ChatID: chat, Anchor: target, Limit: 1}, &before)
	add("link_reaction_capture", rmPut(w, rmObservation("messages.reaction", "rich-reaction", map[string]any{"message_id": rmRef("messages.message", "rich-link"), "sender_handle_id": rmRef("messages.handle", "rich-handle"), "kind": "emoji", "emoji": "🧪", "part_index": 0, "sent_at": at.Add(time.Hour)})) == nil)
	var after contracts.ThreadResult
	err = c.Call(ctx, "messages.thread", contracts.ThreadQuery{ChatID: chat, Anchor: target, Limit: 1}, &after)
	add("reaction_updates_link_thread_live", err == nil && len(after.Items) == 1 && len(after.Items[0].Reactions) == 1 && len(before.Items) == 1 && reflect.DeepEqual(rmData(after.Items[0].Record)["balloon"], rmData(before.Items[0].Record)["balloon"]))
	if len(after.Items) == 1 {
		fields := rmData(after.Items[0].Record)
		fields["subject"] = "Synthetic revised subject"
		add("live_rich_mutation_capture", rmPut(w, rmObservation("messages.message", "rich-link", fields)) == nil)
		var changed contracts.ThreadResult
		err = c.Call(ctx, "messages.thread", contracts.ThreadQuery{ChatID: chat, Anchor: target, Limit: 1}, &changed)
		add("live_rich_mutation_increments_record_version", err == nil && len(changed.Items) == 1 && changed.Items[0].Version > after.Items[0].Version && rmData(changed.Items[0].Record)["subject"] == "Synthetic revised subject" && reflect.DeepEqual(rmData(changed.Items[0].Record)["balloon"], fields["balloon"]))
	}

	richDeletedMessages(ctx, w, add)
	for _, a := range e.Assertions {
		if !a.Passed {
			return e
		}
	}
	e.Passed = true
	return e
}
