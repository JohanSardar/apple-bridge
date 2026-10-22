package scenarios

import (
	"context"
	"fmt"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"os/exec"
	"strings"
	"time"
)

func runMessageSearch(ctx context.Context, s Scenario, level string) Evidence {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e := Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, GitSHA: strings.TrimSpace(string(sha)), At: time.Now().UTC(), Hosts: []string{"air"}, Assertions: []Assertion{}, Metrics: map[string]any{}, Conditions: map[string]string{"dataset": "synthetic search fixture"}}
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	w, err := world.New(ctx, level, 0, 0, world.BlobOptions{Scope: "messages:read,contacts:read,system:read"})
	if err != nil {
		add("world_start", false)
		return e
	}
	defer w.Close()
	err = w.ControlAction("/external-change", map[string]any{"key": "search-contact", "collection": "contacts.contact", "data": map[string]any{"given_name": "Orchard", "family_name": "Keeper", "emails": []any{map[string]any{"value": "search-a@example.com"}}}})
	if err == nil {
		err = w.FlushCaptures(w.Control.Timeout)
	}
	add("search_contact_fixture", err == nil)
	if err != nil {
		e.Metrics["fixture_error"] = err.Error()
		return e
	}
	observations := []contracts.Observation{rmObservation("messages.handle", "search-a", map[string]any{"address": "search-a@example.com", "service": "iMessage"}), rmObservation("messages.handle", "search-b", map[string]any{"address": "search-b@example.com", "service": "iMessage"}), rmObservation("messages.chat", "search-chat", map[string]any{"display_name": "[ab-test] Orchard", "participants": []any{rmRef("messages.handle", "search-a"), rmRef("messages.handle", "search-b")}, "style": "group", "service": "iMessage"})}
	at := time.Date(2026, 10, 20, 12, 0, 0, 0, time.UTC)
	for index := 0; index < 55; index++ {
		key := fmt.Sprintf("search-message-%02d", index)
		sender := "search-a"
		if index%2 == 1 {
			sender = "search-b"
		}
		data := map[string]any{"chat_id": rmRef("messages.chat", "search-chat"), "sender_handle_id": rmRef("messages.handle", sender), "sent_at": at.Add(time.Duration(index) * time.Minute), "kind": "message", "text": fmt.Sprintf("[ab-test] Orchard stone %d", index), "is_from_me": false, "parts": []any{}}
		if index%3 == 0 {
			data["read_at"] = at.Add(time.Hour)
		}
		if index%5 == 2 {
			data["balloon"] = map[string]any{"kind": "link", "link": map[string]any{"url": "https://example.com/orchard", "title": "Cliff lantern"}}
		}
		if index%5 == 3 {
			data["balloon"] = map[string]any{"kind": "find_my", "app": map[string]any{"caption": "Maple location"}}
		}
		observations = append(observations, rmObservation("messages.message", key, data))
		if index%5 < 2 {
			mime := "image/png"
			if index%5 == 1 {
				mime = "application/pdf"
			}
			observations = append(observations, rmObservation("messages.attachment", key+"-attachment", map[string]any{"message_id": rmRef("messages.message", key), "mime_type": mime, "filename": "Canopy.pdf"}))
		}
		if index%5 == 4 {
			observations = append(observations, rmObservation("messages.attachment", key+"-hidden", map[string]any{"message_id": rmRef("messages.message", key), "mime_type": "image/png", "hidden": true, "filename": "Hidden.pdf"}))
		}
	}
	for _, state := range []string{"deleted", "archived", "spam", "spam-chat", "deleted-message", "retracted-message"} {
		key := "search-excluded-" + state
		chat := map[string]any{"display_name": "[ab-test] Orchard excluded", "participants": []any{rmRef("messages.handle", "search-a")}, "style": "direct", "service": "iMessage"}
		message := map[string]any{"chat_id": rmRef("messages.chat", key), "sent_at": at, "kind": "message", "text": "[ab-test] Orchard excluded", "is_from_me": false, "parts": []any{}}
		if state == "deleted" {
			chat["deleted_at"] = at
		}
		if state == "archived" {
			chat["is_archived"] = true
		}
		if state == "spam-chat" {
			chat["filter"] = "spam"
		}
		if state == "deleted-message" {
			message["deleted_at"] = at
		}
		if state == "retracted-message" {
			message["retracted_at"] = at
		}
		if state == "spam" {
			message["is_spam"] = true
		}
		observations = append(observations, rmObservation("messages.chat", key, chat), rmObservation("messages.message", key+"-message", message))
	}
	for start := 0; start < len(observations); start += 64 {
		if err = rmPut(w, observations[start:min(start+64, len(observations))]...); err != nil {
			break
		}
	}
	add("synthetic_search_fixture", err == nil)
	if err != nil {
		e.Metrics["fixture_error"] = err.Error()
		return e
	}
	aliases, err := gatewayAliases(w)
	add("search_aliases", err == nil)
	handle := aliases[aliasKey(contracts.Messages, "messages.handle", "search-a")]
	contact := aliases[aliasKey(contracts.Contacts, "contacts.contact", "search-contact")]
	for _, protocol := range []string{"rest", "mcp"} {
		client := scripted.Client{Endpoint: w.Gateway, Protocol: protocol}
		read := func(q contracts.MessageSearchQuery) (contracts.MessageSearchResult, error) {
			var result contracts.MessageSearchResult
			err := client.Call(ctx, "messages.search", q, &result)
			return result, err
		}
		ids := map[string]bool{}
		after := ""
		pages := 0
		ordered := true
		previous := time.Time{}
		for {
			page, err := read(contracts.MessageSearchQuery{Text: "orch", Limit: 20, After: after})
			if err != nil {
				e.Metrics[protocol+"_pagination_error"] = err.Error()
				ordered = false
				break
			}
			pages++
			for _, item := range page.Items {
				stamp, _ := time.Parse(time.RFC3339Nano, rmData(item.Record)["sent_at"].(string))
				if ids[item.ID] || !previous.IsZero() && stamp.After(previous) {
					ordered = false
				}
				ids[item.ID] = true
				previous = stamp
			}
			if page.Next == "" {
				break
			}
			after = page.Next
			if pages > 4 {
				ordered = false
				break
			}
		}
		add(protocol+"_prefix_pagination_all_55_unique_ordered", ordered && len(ids) == 55 && pages == 3)
		for _, kind := range []string{"photos", "documents", "links", "locations"} {
			page, err := read(contracts.MessageSearchQuery{Text: "orch", Kind: kind, Limit: 200})
			valid := err == nil && len(page.Items) == 11
			for _, item := range page.Items {
				for _, attachment := range item.Attachments {
					valid = valid && rmData(attachment)["hidden"] != true
				}
			}
			add(protocol+"_kind_"+kind+"_before_limit_hidden_excluded", valid)
		}
		from, err := read(contracts.MessageSearchQuery{Text: "orch", FromHandleID: handle, Unread: true, Limit: 200})
		valid := err == nil && len(from.Items) == 18
		for _, item := range from.Items {
			data := rmData(item.Record)
			valid = valid && data["sender_handle_id"] == handle && data["read_at"] == nil
		}
		add(protocol+"_from_and_unread_intersection", valid)
		linked, err := read(contracts.MessageSearchQuery{FromContactID: contact, Limit: 200})
		add(protocol+"_contact_linked_sender", err == nil && len(linked.Items) == 28)
		filename, err := read(contracts.MessageSearchQuery{Text: "Canop", Limit: 200})
		add(protocol+"_attachment_filename_prefix", err == nil && len(filename.Items) == 22)
		for _, term := range []string{"Cliff", "Maple"} {
			page, err := read(contracts.MessageSearchQuery{Text: term, Limit: 200})
			add(protocol+"_balloon_content_"+term, err == nil && len(page.Items) == 11)
		}
		page, err := read(contracts.MessageSearchQuery{Text: "orch", Limit: 1})
		_, changed := read(contracts.MessageSearchQuery{Text: "stone", After: page.Next})
		add(protocol+"_cursor_bound_to_query", err == nil && page.Next != "" && changed != nil)
		for _, q := range []contracts.MessageSearchQuery{{}, {Text: strings.Repeat("x", 513)}, {Text: "orch", Kind: "invalid"}, {Text: "orch", Limit: 201}, {Text: "orch", FromHandleID: handle, FromContactID: contact}} {
			_, err := read(q)
			add(protocol+"_invalid_query_rejected", err != nil)
		}
		page, err = read(contracts.MessageSearchQuery{Text: "orch OR definitelyabsent"})
		add(protocol+"_fts_syntax_is_literal", err == nil && len(page.Items) == 0)
	}
	hidden := w.Gateway
	hidden.Token, _ = w.ScopedToken(w.Gateway, "messages:read")
	var result contracts.MessageSearchResult
	err = (scripted.Client{Endpoint: hidden}).Call(ctx, "messages.search", contracts.MessageSearchQuery{Text: "orch", FromContactID: contact}, &result)
	add("contacts_hidden_sender_lookup_rejected", err != nil)
	hidden.Token, _ = w.ScopedToken(w.Gateway, "contacts:read")
	err = (scripted.Client{Endpoint: hidden}).Call(ctx, "messages.search", contracts.MessageSearchQuery{Text: "orch"}, &result)
	add("messages_visibility", err == nil && len(result.Items) == 0)
	delta := func(body string, deleted any) contracts.Observation {
		return rmObservation("messages.message", "search-message-54", map[string]any{"chat_id": rmRef("messages.chat", "search-chat"), "sender_handle_id": rmRef("messages.handle", "search-a"), "sent_at": at.Add(54 * time.Minute), "kind": "message", "text": body, "is_from_me": false, "parts": []any{}, "deleted_at": deleted})
	}
	add("search_text_delta_capture", rmPut(w, delta("[ab-test] Willow", nil)) == nil)
	migration, err := rmSearchMigration(ctx, w)
	e.Metrics["fts_migration"] = migration
	add("fts_v3_upgrade_preserves_records_and_rebuilds_filename_index", err == nil)
	if err != nil {
		e.Metrics["migration_error"] = err.Error()
		return e
	}
	client := scripted.Client{Endpoint: w.Gateway}
	var updated contracts.MessageSearchResult
	err = client.Call(ctx, "messages.search", contracts.MessageSearchQuery{Text: "orch", Limit: 200}, &updated)
	add("search_text_delta_removes_old_match", err == nil && len(updated.Items) == 54)
	err = client.Call(ctx, "messages.search", contracts.MessageSearchQuery{Text: "will", Limit: 200}, &updated)
	add("search_text_delta_adds_new_match", err == nil && len(updated.Items) == 1)
	add("search_delete_capture", rmPut(w, delta("[ab-test] Willow", at)) == nil)
	err = client.Call(ctx, "messages.search", contracts.MessageSearchQuery{Text: "will", Limit: 200}, &updated)
	add("deleted_message_excluded", err == nil && len(updated.Items) == 0)
	err = client.Call(ctx, "messages.search", contracts.MessageSearchQuery{Text: "Hidden", Limit: 200}, &updated)
	add("hidden_attachment_filename_excluded", err == nil && len(updated.Items) == 0)
	e.Metrics["messages"] = 55
	e.Metrics["pages"] = 3
	e.Passed = true
	for _, assertion := range e.Assertions {
		e.Passed = e.Passed && assertion.Passed
	}
	return e
}
