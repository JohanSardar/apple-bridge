package scenarios

import (
	"context"
	"fmt"
	"os/exec"
	"reflect"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func runMediaCollection(ctx context.Context, s Scenario, level string) Evidence {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e := Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, GitSHA: strings.TrimSpace(string(sha)), At: time.Now().UTC(), Assertions: []Assertion{}, Latency: map[string]Latency{}, Hosts: []string{"air"}, Conditions: map[string]string{"dataset": "synthetic SimApplePeer only", "transport": "REST and MCP"}}
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	w, err := world.New(ctx, level, 0, 0, world.BlobOptions{Scope: "messages:read,system:read"})
	add("world_start", err == nil)
	if err != nil {
		return e
	}
	defer w.Close()
	at := time.Date(2026, 5, 1, 0, 0, 0, 0, time.UTC)
	records := []contracts.Observation{rmObservation("messages.chat", "rm-chat-000", map[string]any{"display_name": "Bridge Test Media"})}
	for i := range 9 {
		key := fmt.Sprintf("media-message-%d", i)
		mime := "image/png"
		if i%2 == 1 {
			mime = "video/quicktime"
		}
		extra := map[string]any{}
		if i == 7 {
			extra["deleted_at"] = at.Add(time.Hour)
		}
		records = append(records, rmMessage(key, at.Add(time.Duration(i)*time.Second), extra), rmObservation("messages.attachment", fmt.Sprintf("media-attachment-%d", i), map[string]any{"message_id": rmRef("messages.message", key), "mime_type": mime, "filename": fmt.Sprintf("synthetic-%d", i), "hidden": i == 8}))
	}
	err = rmPut(w, records...)
	add("interleaved_synthetic_sequence_committed", err == nil)
	if err != nil {
		return e
	}
	aliases, err := gatewayAliases(w)
	add("aliases_resolved", err == nil)
	if err != nil {
		return e
	}
	chatID := aliases[aliasKey(contracts.Messages, "messages.chat", "rm-chat-000")]
	expected := []string{}
	for i := 6; i >= 0; i-- {
		expected = append(expected, aliases[aliasKey(contracts.Messages, "messages.attachment", fmt.Sprintf("media-attachment-%d", i))])
	}
	var parity []contracts.Record
	for _, protocol := range []string{"rest", "mcp"} {
		c := scripted.Client{Endpoint: w.Gateway, Protocol: protocol}
		var page contracts.CollectionResult
		query := contracts.CollectionQuery{ChatID: chatID, Kind: "media", Limit: 2}
		start := time.Now()
		err = c.Call(ctx, "messages.collection", query, &page)
		add(protocol+"_media_kind_accepted", err == nil)
		if err != nil {
			continue
		}
		all := append([]contracts.Record{}, page.Items...)
		firstCursor := page.Next
		liveKey := "media-live-" + protocol
		err = rmPut(w, rmMessage(liveKey, at.Add(time.Hour), nil), rmObservation("messages.attachment", liveKey+"-attachment", map[string]any{"message_id": rmRef("messages.message", liveKey), "mime_type": "video/mp4", "filename": liveKey}))
		valid := err == nil
		for pages := 0; page.Next != "" && pages < 10; pages++ {
			query.After = page.Next
			page = contracts.CollectionResult{}
			err = c.Call(ctx, "messages.collection", query, &page)
			valid = valid && err == nil && len(page.Items) <= 2
			all = append(all, page.Items...)
		}
		ids := []string{}
		for _, r := range all {
			ids = append(ids, r.ID)
		}
		add(protocol+"_cursor_paging_no_gaps_or_duplicates_during_live_insert", valid && page.Next == "" && reflect.DeepEqual(ids, expected))
		geometry := true
		for i, r := range all {
			n := 6 - i
			sent, parseErr := time.Parse(time.RFC3339Nano, fmt.Sprint(rmData(r)["sent_at"]))
			geometry = geometry && parseErr == nil && rmData(r)["message_id"] == aliases[aliasKey(contracts.Messages, "messages.message", fmt.Sprintf("media-message-%d", n))] && sent.Equal(at.Add(time.Duration(n)*time.Second))
		}
		add(protocol+"_owning_message_and_sent_at_projected", geometry)
		if protocol == "rest" {
			parity = all
		} else {
			add("rest_mcp_items_and_projection_identical", reflect.DeepEqual(parity, all))
		}
		var fresh contracts.CollectionResult
		err = c.Call(ctx, "messages.collection", contracts.CollectionQuery{ChatID: chatID, Kind: "media", Limit: 1}, &fresh)
		add(protocol+"_live_insert_visible_at_head", err == nil && len(fresh.Items) == 1 && rmData(fresh.Items[0])["filename"] == liveKey)
		err = c.Call(ctx, "messages.collection", contracts.CollectionQuery{ChatID: chatID, Kind: "photos", After: firstCursor}, &fresh)
		add(protocol+"_cursor_bound_to_kind", err != nil)
		err = c.Call(ctx, "messages.collection", contracts.CollectionQuery{ChatID: chatID, Kind: "media", Limit: 201}, &fresh)
		add(protocol+"_page_limit_enforced", err != nil)
		token, tokenErr := w.ScopedToken(w.Gateway, "system:read")
		restricted := w.Gateway
		restricted.Token = token
		err = (scripted.Client{Endpoint: restricted, Protocol: protocol}).Call(ctx, "messages.collection", contracts.CollectionQuery{ChatID: chatID, Kind: "media"}, &fresh)
		add(protocol+"_visibility_preserved", tokenErr == nil && err == nil && len(fresh.Items) == 0)
		elapsed := float64(time.Since(start)) / float64(time.Millisecond)
		e.Latency[protocol] = stats([]float64{elapsed}, 0)
		add(protocol+"_journey_under_one_second", elapsed < 1000)
		err = rmPut(w, rmMessage(liveKey, at.Add(time.Hour), map[string]any{"deleted_at": at.Add(2 * time.Hour)}))
		add(protocol+"_live_fixture_hidden_before_parity_repeat", err == nil)
	}
	e.Passed = true
	for _, a := range e.Assertions {
		e.Passed = e.Passed && a.Passed
	}
	return e
}
