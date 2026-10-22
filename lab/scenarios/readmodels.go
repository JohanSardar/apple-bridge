package scenarios

import (
	"context"
	"database/sql"
	"encoding/json"
	"fmt"
	"net/http"
	"os/exec"
	"path/filepath"
	"reflect"
	"slices"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func rmObservation(collection, key string, data map[string]any) contracts.Observation {
	return contracts.Observation{Kind: "upsert", Collection: collection, NativeKey: key, Data: mustJSON(data)}
}
func rmRef(collection, key string) any {
	return map[string]any{"$ref": map[string]string{"collection": collection, "native_key": key}}
}
func rmData(r contracts.Record) map[string]any {
	var d map[string]any
	json.Unmarshal(r.Data, &d)
	return d
}
func rmPut(w *world.World, records ...contracts.Observation) error {
	return w.ControlAction("/readmodels", map[string]any{"action": "put", "records": records})
}
func rmChat(ctx context.Context, c scripted.Client, id string) (contracts.ChatSummary, error) {
	var result contracts.ChatsResult
	err := c.Call(ctx, "messages.chats", contracts.ChatsQuery{Limit: 200}, &result)
	for _, s := range result.Chats {
		if s.Chat.ID == id {
			return s, err
		}
	}
	return contracts.ChatSummary{}, fmt.Errorf("chat missing: %w", err)
}
func rmMessage(key string, at time.Time, extra map[string]any) contracts.Observation {
	d := map[string]any{"chat_id": rmRef("messages.chat", "rm-chat-000"), "sent_at": at, "kind": "message", "text": "[ab-test] " + key, "is_from_me": false, "parts": []any{}}
	for k, v := range extra {
		d[k] = v
	}
	return rmObservation("messages.message", key, d)
}
func runReadModels(ctx context.Context, s Scenario, level string) Evidence {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e := Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, GitSHA: strings.TrimSpace(string(sha)), At: time.Now().UTC(), Assertions: []Assertion{}, Latency: map[string]Latency{}, Hosts: []string{"air"}, Conditions: map[string]string{"dataset": "synthetic SimApplePeer only", "transport": "loopback HTTP REST and MCP", "read_state": "warm SQLite WAL, normal concurrent host activity"}, Metrics: map[string]any{}}
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	w, err := world.New(ctx, level, 0, 0, world.BlobOptions{Scope: "messages:read,contacts:read,system:read"})
	if err != nil {
		add("world_start", false)
		return e
	}
	defer w.Close()
	w.Control.Timeout = 10 * time.Minute
	err = w.ControlAction("/readmodels", map[string]any{"action": "seed", "count": 1000})
	add("synthetic_sim_seed_committed", err == nil)
	if err != nil {
		return e
	}
	observations, err := w.Inspect()
	if err != nil {
		add("independent_truth", false)
		return e
	}
	aliases, err := gatewayAliases(w)
	if err != nil {
		add("bridge_aliases", false)
		return e
	}
	observations = slices.DeleteFunc(observations, func(o contracts.Observation) bool {
		return contracts.CollectionSource(o.Collection) != contracts.Messages
	})
	truth, err := truthRecords(observations, aliases)
	if err != nil {
		add("independent_truth_resolved", false)
		return e
	}
	chatID := ""
	expected := []contracts.Record{}
	for _, r := range truth {
		if r.Collection == "messages.chat" && rmData(r)["display_name"] == "Bridge Test 000" {
			chatID = r.ID
		}
	}
	for _, r := range truth {
		if r.Collection == "messages.message" && rmData(r)["chat_id"] == chatID {
			expected = append(expected, r)
		}
	}
	slices.SortFunc(expected, func(a, b contracts.Record) int {
		ad, bd := rmData(a), rmData(b)
		cmp := strings.Compare(ad["sent_at"].(string), bd["sent_at"].(string))
		if cmp == 0 {
			return strings.Compare(a.ID, b.ID)
		}
		return cmp
	})
	add("truth_has_500_chats_and_501_thread_messages", chatID != "" && len(expected) == 501)
	if len(expected) != 501 {
		return e
	}
	for _, protocol := range []string{"rest", "mcp"} {
		c := scripted.Client{Endpoint: w.Gateway, Protocol: protocol}
		var chats contracts.ChatsResult
		err = c.Call(ctx, "messages.chats", contracts.ChatsQuery{Limit: 200}, &chats)
		add(protocol+"_chat_summaries", err == nil && len(chats.Chats) == 200 && chats.Chats[0].Chat.ID == chatID && chats.Chats[0].UnreadCount == 0 && chats.Chats[0].LastMessage.ID == expected[len(expected)-1].ID && chats.Chats[0].Muted == nil && chats.Chats[0].PinnedIndex == nil && !chats.Chats[0].HasUnseenMention && len(chats.CoverageNotes) > 0)
		all := []contracts.ChatSummary{}
		seen := map[string]bool{}
		valid := err == nil
		for {
			for _, row := range chats.Chats {
				valid = valid && !seen[row.Chat.ID]
				seen[row.Chat.ID] = true
				all = append(all, row)
			}
			if chats.Next == "" {
				break
			}
			next := chats.Next
			chats = contracts.ChatsResult{}
			err = c.Call(ctx, "messages.chats", contracts.ChatsQuery{Limit: 200, After: next}, &chats)
			if err != nil {
				valid = false
				break
			}
		}
		for i := 1; i < len(all); i++ {
			a, b := all[i-1], all[i]
			valid = valid && (a.LastActivityAt > b.LastActivityAt || a.LastActivityAt == b.LastActivityAt && a.Chat.ID < b.Chat.ID)
		}
		add(protocol+"_all_chats_stably_ordered", valid && len(all) == 500)
		var thread contracts.ThreadResult
		err = c.Call(ctx, "messages.thread", contracts.ThreadQuery{ChatID: chatID, Limit: 100}, &thread)
		add(protocol+"_latest_thread_matches_truth", err == nil && len(thread.Items) == 100 && thread.Items[99].ID == expected[len(expected)-1].ID && thread.Before != "" && thread.After == "")
	}
	denied := w.Gateway
	denied.Token, err = w.ScopedToken(w.Gateway, "system:read")
	add("restricted_token", err == nil)
	for _, protocol := range []string{"rest", "mcp"} {
		c := scripted.Client{Endpoint: denied, Protocol: protocol}
		var chats contracts.ChatsResult
		err = c.Call(ctx, "messages.chats", contracts.ChatsQuery{}, &chats)
		add(protocol+"_visibility_chats", err == nil && len(chats.Chats) == 0)
		var t contracts.ThreadResult
		err = c.Call(ctx, "messages.thread", contracts.ThreadQuery{ChatID: chatID}, &t)
		add(protocol+"_visibility_thread", err == nil && len(t.Items) == 0)
		var a contracts.CollectionResult
		err = c.Call(ctx, "messages.collection", contracts.CollectionQuery{ChatID: chatID, Kind: "photos"}, &a)
		add(protocol+"_visibility_collection", err == nil && len(a.Items) == 0)
	}
	if s.ID == "WEB-RM-THREAD" {
		rmPaging(ctx, w, chatID, expected, add)
		rmPlacedStickers(ctx, w, add)
		rmRecoverableMessages(ctx, w, add)
	}
	if s.ID == "WEB-RM-CHATS" {
		rmNames(ctx, w, chatID, add)
	}
	rmLive(ctx, w, chatID, add)
	rmBoundaries(ctx, w, chatID, add)
	if s.ID == "WEB-RM-CHATS" {
		rmPreviews(ctx, w, chatID, add)
		rmRichFilters(ctx, w, chatID, add)
		rmPinPreview(ctx, w, add)
	}
	if s.ID == "WEB-RM-CHATS" {
		add("replica_added_after_data_for_snapshot", w.AddReplicaScope("messages:read,contacts:read,system:read") == nil)
		if len(w.Replicas) > 0 {
			var want, got contracts.ChatsResult
			c := scripted.Client{Endpoint: w.Gateway}
			c.Call(ctx, "messages.chats", contracts.ChatsQuery{Limit: 200}, &want)
			err = w.Await(20*time.Second, func() bool {
				err := (scripted.Client{Endpoint: w.Replicas[0]}).Call(ctx, "messages.chats", contracts.ChatsQuery{Limit: 200}, &got)
				return err == nil && reflect.DeepEqual(want.Chats, got.Chats)
			})
			if err != nil {
				wanted, actual := map[string]contracts.ChatSummary{}, map[string]contracts.ChatSummary{}
				for _, row := range want.Chats {
					wanted[row.Chat.ID] = row
				}
				for _, row := range got.Chats {
					actual[row.Chat.ID] = row
				}
				missing, extra := 0, 0
				differing := map[string]int{}
				for id, row := range wanted {
					other, ok := actual[id]
					if !ok {
						missing++
						continue
					}
					left, right := map[string]any{}, map[string]any{}
					json.Unmarshal(mustJSON(row), &left)
					json.Unmarshal(mustJSON(other), &right)
					for field, value := range left {
						if !reflect.DeepEqual(value, right[field]) {
							differing[field]++
						}
					}
				}
				for id := range actual {
					if _, ok := wanted[id]; !ok {
						extra++
					}
				}
				e.Metrics["snapshot_rebuild_diagnostic"] = map[string]any{"gateway_count": len(want.Chats), "replica_count": len(got.Chats), "missing": missing, "extra": extra, "different_fields": differing, "gateway_sequence": want.Cursor.Sequence, "replica_sequence": got.Cursor.Sequence}
			}
			add("snapshot_rebuild_matches_gateway", err == nil)
			rmPut(w, rmMessage("rm-live-replica", time.Date(2026, 6, 1, 0, 0, 0, 0, time.UTC), nil))
			c.Call(ctx, "messages.chats", contracts.ChatsQuery{Limit: 200}, &want)
			err = w.Await(20*time.Second, func() bool {
				err := (scripted.Client{Endpoint: w.Replicas[0], Protocol: "mcp"}).Call(ctx, "messages.chats", contracts.ChatsQuery{Limit: 200}, &got)
				return err == nil && reflect.DeepEqual(want.Chats, got.Chats)
			})
			add("replica_live_apply_matches_gateway", err == nil)
		}
		rmMigration(ctx, w, chatID, add)
	} else {
		err = w.ControlAction("/readmodels", map[string]any{"action": "seed", "count": 170000})
		add("170000_message_sim_seed_committed", err == nil)
		if err != nil {
			return e
		}
	}
	for _, protocol := range []string{"rest", "mcp"} {
		c := scripted.Client{Endpoint: w.Gateway, Protocol: protocol}
		rateLimited := 0
		c.Observe = func(_ http.Header, raw []byte) {
			if len(raw) <= 32 && strings.TrimSpace(string(raw)) == "rate limited" {
				rateLimited++
			}
		}
		durations := []float64{}
		ok := true
		for i := 0; i < 100; i++ {
			if s.ID == "WEB-RM-CHATS" {
				time.Sleep(10 * time.Millisecond)
			}
			start := time.Now()
			if s.ID == "WEB-RM-CHATS" {
				var r contracts.ChatsResult
				err = c.Call(ctx, "messages.chats", contracts.ChatsQuery{Limit: 50}, &r)
				ok = ok && err == nil && len(r.Chats) == 50
			} else {
				var r contracts.ThreadResult
				err = c.Call(ctx, "messages.thread", contracts.ThreadQuery{ChatID: chatID, Limit: 100}, &r)
				ok = ok && err == nil && len(r.Items) == 100
			}
			durations = append(durations, float64(time.Since(start))/float64(time.Millisecond))
		}
		name := protocol + "_" + strings.ToLower(strings.TrimPrefix(s.ID, "WEB-RM-"))
		latency := stats(durations, 0)
		e.Latency[name] = latency
		e.Metrics[protocol+"_rate_limited_samples"] = rateLimited
		budget := 15.0
		if s.ID == "WEB-RM-THREAD" {
			budget = 10
		}
		if s.ID == "WEB-RM-CHATS" {
			add(name+"_requests_succeed", ok)
		}
		add(name+"_p95_budget", ok && latency.P95 <= budget)
	}
	if s.ID == "WEB-RM-CHATS" {
		if err := rmExactParticipants(ctx, w, add); err != nil {
			e.Metrics["exact_participant_aliases_error"] = err.Error()
		}
	}
	if s.ID == "WEB-RM-CHATS" {
		e.Metrics["latency_sample_gap_ms"] = 10
	}
	e.Metrics["chats"] = 500
	if s.ID == "WEB-RM-THREAD" {
		e.Metrics["seed_messages"] = 170000
	} else {
		e.Metrics["seed_messages"] = 1000
	}
	e.Passed = true
	for _, a := range e.Assertions {
		e.Passed = e.Passed && a.Passed
	}
	return e
}

func rmPaging(ctx context.Context, w *world.World, chatID string, expected []contracts.Record, add func(string, bool)) {
	original := map[string]bool{}
	for _, r := range expected {
		original[r.ID] = true
	}
	for _, protocol := range []string{"rest", "mcp"} {
		c := scripted.Client{Endpoint: w.Gateway, Protocol: protocol}
		for _, direction := range []string{"forward", "backward"} {
			seen := map[string]bool{}
			ok := true
			query := contracts.ThreadQuery{ChatID: chatID, Anchor: expected[0].ID, Limit: 1}
			if direction == "backward" {
				query = contracts.ThreadQuery{ChatID: chatID, Limit: 37}
			}
			done := make(chan error, 1)
			for page := 0; page < 40; page++ {
				var result contracts.ThreadResult
				err := c.Call(ctx, "messages.thread", query, &result)
				if err != nil {
					ok = false
					break
				}
				if page == 0 {
					go func() {
						records := []contracts.Observation{}
						for i := 0; i < 8; i++ {
							records = append(records, rmMessage(fmt.Sprintf("concurrent-%s-%s-%d", protocol, direction, i), time.Date(2026, 3, 1, 0, 0, i, 0, time.UTC), map[string]any{"is_from_me": true}))
						}
						done <- rmPut(w, records...)
					}()
				}
				for _, r := range result.Items {
					if seen[r.ID] {
						ok = false
					}
					seen[r.ID] = true
				}
				query = contracts.ThreadQuery{ChatID: chatID, Limit: 37}
				if direction == "forward" {
					query.After = result.After
					if query.After == "" {
						break
					}
				} else {
					query.Before = result.Before
					if query.Before == "" {
						break
					}
				}
			}
			select {
			case err := <-done:
				ok = ok && err == nil
			case <-ctx.Done():
				ok = false
			}
			for id := range original {
				ok = ok && seen[id]
			}
			add(protocol+"_"+direction+"_concurrent_paging_no_gaps_or_duplicates", ok)
		}
	}
}
func rmLive(ctx context.Context, w *world.World, chatID string, add func(string, bool)) {
	at := time.Date(2026, 5, 1, 0, 0, 0, 0, time.UTC)
	root := rmMessage("rm-root", at, map[string]any{"is_from_me": true})
	unread := rmMessage("rm-unread", at.Add(time.Second), nil)
	reply := rmMessage("rm-reply", at.Add(2*time.Second), map[string]any{"is_from_me": true, "thread_originator_id": rmRef("messages.message", "rm-root")})
	reaction := func(key, kind string, removed bool, second int, extra map[string]any) contracts.Observation {
		d := map[string]any{"message_id": rmRef("messages.message", "rm-root"), "sender_handle_id": "synthetic-sender", "part_index": 0, "kind": kind, "removed": removed, "sent_at": at.Add(time.Duration(second) * time.Second)}
		for k, v := range extra {
			d[k] = v
		}
		return rmObservation("messages.reaction", key, d)
	}
	records := []contracts.Observation{root, unread, reply, rmMessage("rm-tail", at.Add(3*time.Second), map[string]any{"is_from_me": true}), reaction("rm-love", "love", false, 3, nil), reaction("rm-like", "like", false, 4, nil), reaction("rm-old-remove", "love", true, 5, nil), reaction("rm-emoji", "emoji", false, 6, map[string]any{"emoji": "🧪"}), reaction("rm-emoji-removed", "emoji", true, 7, map[string]any{"emoji": "🧪"}), reaction("rm-other-emoji", "emoji", false, 8, map[string]any{"emoji": "🎉"}), reaction("rm-other-sender", "question", false, 9, map[string]any{"sender_handle_id": "synthetic-other"}), reaction("rm-sticker-one", "sticker", false, 6, map[string]any{"sticker_id": "one"}), reaction("rm-sticker-one-latest", "sticker", false, 7, map[string]any{"sticker_id": "one"}), reaction("rm-sticker-two", "sticker", false, 8, map[string]any{"sticker_id": "two"})}
	for i, mime := range []string{"image/png", "video/mp4", "audio/aac", "application/pdf", "image/png"} {
		records = append(records, rmObservation("messages.attachment", fmt.Sprintf("rm-attachment-%d", i), map[string]any{"message_id": rmRef("messages.message", "rm-root"), "mime_type": mime, "filename": "synthetic", "is_sticker": i == 4, "derivatives": map[string]any{"thumb": "synthetic-digest"}, "media": map[string]any{"width": 64}}))
	}
	err := rmPut(w, records...)
	add("inbound_reaction_attachment_reply_capture", err == nil)
	aliases, _ := gatewayAliases(w)
	rootID := aliases[aliasKey(contracts.Messages, "messages.message", "rm-root")]
	unreadID := aliases[aliasKey(contracts.Messages, "messages.message", "rm-unread")]
	for _, protocol := range []string{"rest", "mcp"} {
		c := scripted.Client{Endpoint: w.Gateway, Protocol: protocol}
		summary, err := rmChat(ctx, c, chatID)
		add(protocol+"_live_reaction_preview_and_unread", err == nil && summary.UnreadCount == 1 && summary.LastMessage != nil && summary.LastMessage.Preview.Kind == "reaction" && summary.LastMessage.Preview.Reaction.TargetExcerpt == "[ab-test] rm-root")
		var t contracts.ThreadResult
		err = c.Call(ctx, "messages.thread", contracts.ThreadQuery{ChatID: chatID, Anchor: "unread", Limit: 5}, &t)
		centered := err == nil && t.UnreadAnchor == unreadID && len(t.Items) == 5 && t.Items[2].ID == unreadID
		add(protocol+"_unread_anchor_centered", centered)
		err = c.Call(ctx, "messages.thread", contracts.ThreadQuery{ChatID: chatID, Anchor: rootID, Limit: 1}, &t)
		folded := err == nil && len(t.Items) == 1 && t.Items[0].ID == rootID && len(t.Items[0].Reactions) == 5 && t.Items[0].ReplyCount == 1 && len(t.Items[0].Attachments) == 5
		if folded {
			for _, a := range t.Items[0].Attachments {
				d := rmData(a)
				folded = folded && d["derivatives"] != nil && d["media"] != nil
			}
			kinds := []string{}
			for _, r := range t.Items[0].Reactions {
				kinds = append(kinds, rmData(r)["kind"].(string))
			}
			slices.Sort(kinds)
			folded = folded && reflect.DeepEqual(kinds, []string{"emoji", "like", "question", "sticker", "sticker"})
		}
		add(protocol+"_explicit_anchor_folded_reactions_full_attachments_reply_count", folded)
		for _, kind := range []string{"photos", "videos", "audio", "documents", "links", "locations"} {
			var result contracts.CollectionResult
			err = c.Call(ctx, "messages.collection", contracts.CollectionQuery{ChatID: chatID, Kind: kind}, &result)
			want := 1
			if kind == "links" || kind == "locations" {
				want = 0
			}
			valid := err == nil && len(result.Items) == want
			add(protocol+"_collection_"+kind, valid)
		}
	}
	add("classic_removal_capture", rmPut(w, reaction("rm-like-remove", "like", true, 10, nil), reaction("rm-sticker-remove", "sticker", true, 11, map[string]any{"sticker_id": "one"})) == nil)
	for _, protocol := range []string{"rest", "mcp"} {
		var t contracts.ThreadResult
		err := (scripted.Client{Endpoint: w.Gateway, Protocol: protocol}).Call(ctx, "messages.thread", contracts.ThreadQuery{ChatID: chatID, Anchor: rootID, Limit: 1}, &t)
		add(protocol+"_classic_and_sticker_removal_folded", err == nil && len(t.Items) == 1 && len(t.Items[0].Reactions) == 3)
	}
	read := rmMessage("rm-unread", at.Add(time.Second), map[string]any{"read_at": at.Add(time.Minute)})
	edited := rmMessage("rm-reply", at.Add(20*time.Second), map[string]any{"is_from_me": true, "edited_at": at.Add(time.Minute), "text": "[ab-test] edited", "thread_originator_id": rmRef("messages.message", "rm-root")})
	add("read_edit_capture", rmPut(w, read, edited) == nil)
	for _, protocol := range []string{"rest", "mcp"} {
		c := scripted.Client{Endpoint: w.Gateway, Protocol: protocol}
		summary, err := rmChat(ctx, c, chatID)
		add(protocol+"_read_edit_live_summary", err == nil && summary.UnreadCount == 0 && summary.LastMessage != nil && summary.LastMessage.Preview.Kind == "edited" && summary.LastMessage.Preview.Text == "[ab-test] edited")
		var t contracts.ThreadResult
		err = c.Call(ctx, "messages.thread", contracts.ThreadQuery{ChatID: chatID, Anchor: "unread", Limit: 5}, &t)
		add(protocol+"_unread_falls_back_to_latest", err == nil && t.UnreadAnchor == "" && len(t.Items) == 5 && t.After == "")
	}
}
func rmMigration(ctx context.Context, w *world.World, chatID string, add func(string, bool)) {
	c := scripted.Client{Endpoint: w.Gateway}
	before, err := rmChat(ctx, c, chatID)
	if err != nil {
		add("migration_baseline", false)
		return
	}
	w.StopGateway()
	db, err := sql.Open("sqlite", filepath.Join(w.Gateway.Home, "state.db"))
	if err != nil {
		add("migration_open", false)
		return
	}
	_, err = db.Exec("DROP TRIGGER rm_msg_insert; DROP TRIGGER rm_msg_delete; DROP TRIGGER rm_activity_insert; DROP TRIGGER rm_activity_delete; DROP TABLE msg_by_chat; DROP TABLE reaction_by_message; DROP TABLE attachment_by_message; DROP TABLE activity_by_chat; DROP TABLE chat_summary; DELETE FROM meta WHERE key='readmodels_version'")
	db.Close()
	add("existing_store_without_read_models", err == nil)
	err = w.RestartGateway()
	add("existing_store_migration_restart", err == nil)
	after, err := rmChat(ctx, c, chatID)
	add("one_time_migration_rebuild_matches", err == nil && reflect.DeepEqual(before, after))
}

func rmNames(ctx context.Context, w *world.World, chatID string, add func(string, bool)) {
	handles := []contracts.Observation{}
	participants := []any{}
	for i, name := range []string{"Alpha", "Beta", "Gamma", "Delta"} {
		address := fmt.Sprintf("synthetic-%d@example.invalid", i)
		err := w.ControlAction("/external-change", map[string]any{"key": fmt.Sprintf("rm-contact-%d", i), "collection": "contacts.contact", "data": map[string]any{"given_name": name, "emails": []any{map[string]any{"value": address}}}})
		add(fmt.Sprintf("contact_name_fixture_%d", i), err == nil)
		key := fmt.Sprintf("rm-handle-%d", i)
		handles = append(handles, rmObservation("messages.handle", key, map[string]any{"address": address, "service": "iMessage"}))
		participants = append(participants, rmRef("messages.handle", key))
	}
	add("contact_fixtures_committed", w.FlushCaptures(20*time.Second) == nil)
	chat := func(display string, people []any) contracts.Observation {
		return rmObservation("messages.chat", "rm-chat-000", map[string]any{"display_name": display, "participants": people, "style": "group", "service": "iMessage"})
	}
	handles = append(handles, chat("", participants))
	add("handle_contact_title_capture", rmPut(w, handles...) == nil)
	rmHandles(ctx, w, add)
	rmKnownSender(ctx, w, chatID, add)
	add("known_sender_original_title_fixture_restored", rmPut(w, chat("", participants)) == nil)
	messageOnly := w.Gateway
	token, err := w.ScopedToken(w.Gateway, "messages:read")
	messageOnly.Token = token
	add("messages_only_token", err == nil)
	for _, protocol := range []string{"rest", "mcp"} {
		row, err := rmChat(ctx, scripted.Client{Endpoint: w.Gateway, Protocol: protocol}, chatID)
		valid := err == nil && row.Title == "Alpha, Beta & 2 others" && len(row.AvatarHandles) == 3
		for _, h := range row.AvatarHandles {
			valid = valid && h.ContactID != nil
		}
		add(protocol+"_contact_names_and_three_avatar_handles", valid)
		row, err = rmChat(ctx, scripted.Client{Endpoint: messageOnly, Protocol: protocol}, chatID)
		valid = err == nil && strings.Contains(row.Title, "synthetic-0@example.invalid")
		for _, h := range row.AvatarHandles {
			valid = valid && h.ContactID == nil
		}
		add(protocol+"_contact_visibility_falls_back_to_addresses", valid)
	}
	add("display_name_override_capture", rmPut(w, chat("Bridge Test 000", participants)) == nil)
	row, err := rmChat(ctx, scripted.Client{Endpoint: w.Gateway}, chatID)
	add("display_name_overrides_participant_names", err == nil && row.Title == "Bridge Test 000")
	add("restore_chat_fixture", rmPut(w, chat("Bridge Test 000", []any{})) == nil)
}

func rmBoundaries(ctx context.Context, w *world.World, chatID string, add func(string, bool)) {
	for _, protocol := range []string{"rest", "mcp"} {
		c := scripted.Client{Endpoint: w.Gateway, Protocol: protocol}
		for i, probe := range []struct {
			name  string
			input any
		}{
			{"messages.chats", map[string]any{"limit": 201}},
			{"messages.chats", map[string]any{"after": "broken"}},
			{"messages.thread", map[string]any{"chat_id": chatID, "before": "broken"}},
			{"messages.thread", map[string]any{"chat_id": chatID, "anchor": "latest", "after": "broken"}},
			{"messages.collection", map[string]any{"chat_id": chatID, "kind": "unknown"}},
		} {
			err := c.Call(ctx, probe.name, probe.input, nil)
			add(fmt.Sprintf("%s_invalid_read_model_input_%d", protocol, i), err != nil)
		}
	}
}
func rmPreviews(ctx context.Context, w *world.World, chatID string, add func(string, bool)) {
	at := time.Date(2026, 5, 2, 0, 0, 0, 0, time.UTC)
	for i, kind := range []string{"text", "attachments", "group_event", "unsent"} {
		extra := map[string]any{"is_from_me": true, "sender_handle_id": "synthetic-sender"}
		if kind == "group_event" {
			extra["kind"] = "group_event"
			extra["group_event"] = map[string]any{"kind": "rename", "name": "Bridge Test"}
		}
		if kind == "unsent" {
			extra["retracted_at"] = at
			extra["retracted_parts"] = []any{0}
		}
		records := []contracts.Observation{rmMessage("rm-preview", at.Add(time.Duration(i)*time.Second), extra)}
		if kind == "attachments" {
			for j, mime := range []string{"image/png", "video/mp4"} {
				records = append(records, rmObservation("messages.attachment", fmt.Sprintf("rm-preview-attachment-%d", j), map[string]any{"message_id": rmRef("messages.message", "rm-preview"), "mime_type": mime}))
			}
		}
		add("preview_"+kind+"_capture", rmPut(w, records...) == nil)
		for _, protocol := range []string{"rest", "mcp"} {
			row, err := rmChat(ctx, scripted.Client{Endpoint: w.Gateway, Protocol: protocol}, chatID)
			ok := err == nil && row.LastMessage != nil && row.LastMessage.Preview.Kind == kind
			if (kind == "group_event" || kind == "unsent") && ok {
				ok = row.LastMessage.SenderHandleID != nil && *row.LastMessage.SenderHandleID == "synthetic-sender"
				if kind == "group_event" {
					ok = ok && row.LastMessage.Preview.GroupEvent["kind"] == "rename"
				}
			}
			if kind == "attachments" && ok {
				ok = row.LastMessage.Preview.Count == 2 && len(row.LastMessage.Preview.AttachmentKinds) == 2
			}
			add(protocol+"_preview_"+kind, ok)
		}
	}
}
