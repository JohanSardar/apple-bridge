package scenarios

import (
	"context"
	"slices"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func checkThreadOriginator(ctx context.Context, w *world.World, fixture threadFixture, e *Evidence, add func(string, bool), budgets bool) {
	want := append([]string{fixture.root}, fixture.replies...)
	ids := func(items []contracts.ThreadItem) []string {
		result := []string{}
		for _, item := range items {
			result = append(result, item.ID)
		}
		return result
	}
	for _, protocol := range []string{"rest", "mcp"} {
		endpoint := w.Gateway
		token, tokenErr := w.ScopedToken(endpoint, "messages:read,contacts:read,system:read")
		add(protocol+"_read_token", tokenErr == nil)
		if tokenErr != nil {
			continue
		}
		endpoint.Token = token
		client := scripted.Client{Endpoint: endpoint, Protocol: protocol}
		query := map[string]any{"chat_id": fixture.chat, "thread_originator_id": fixture.root, "anchor": fixture.root, "limit": 200}
		var first contracts.ThreadResult
		err := client.Call(ctx, "messages.thread", query, &first)
		add(protocol+"_root_and_first_199_distant_replies", err == nil && len(first.Items) == 200 && slices.Equal(ids(first.Items), want[:200]) && first.Items[0].ReplyCount == 205 && first.Before == "" && first.After != "")
		if err != nil || len(first.Items) != 200 {
			continue
		}
		var oldest contracts.ThreadResult
		err = client.Call(ctx, "messages.thread", map[string]any{"chat_id": fixture.chat, "thread_originator_id": fixture.root, "anchor": "oldest", "limit": 200}, &oldest)
		add(protocol+"_filtered_oldest_anchor", err == nil && slices.Equal(ids(oldest.Items), want[:200]) && oldest.Before == "" && oldest.After != "")
		var second contracts.ThreadResult
		err = client.Call(ctx, "messages.thread", map[string]any{"chat_id": fixture.chat, "thread_originator_id": fixture.root, "after": first.After, "limit": 200}, &second)
		add(protocol+"_bounded_forward_page", err == nil && slices.Equal(ids(second.Items), want[200:]) && second.Before != "" && second.After == "")
		var back contracts.ThreadResult
		err = client.Call(ctx, "messages.thread", map[string]any{"chat_id": fixture.chat, "thread_originator_id": fixture.root, "before": second.Before, "limit": 200}, &back)
		add(protocol+"_reverse_page_without_duplicates", err == nil && slices.Equal(ids(back.Items), want[:200]))
		var invalid contracts.ThreadResult
		err = client.Call(ctx, "messages.thread", map[string]any{"chat_id": fixture.chat, "after": first.After}, &invalid)
		add(protocol+"_filtered_cursor_rejected_without_filter", err != nil)
		err = client.Call(ctx, "messages.thread", map[string]any{"chat_id": fixture.chat, "thread_originator_id": fixture.replies[0], "after": first.After}, &invalid)
		add(protocol+"_filtered_cursor_rejected_for_other_root", err != nil)
		err = client.Call(ctx, "messages.thread", map[string]any{"chat_id": fixture.chat, "thread_originator_id": fixture.root, "limit": 201}, &invalid)
		add(protocol+"_page_limit_200", err != nil)
		err = client.Call(ctx, "messages.thread", map[string]any{"chat_id": fixture.otherChat, "thread_originator_id": fixture.root}, &invalid)
		add(protocol+"_root_in_other_chat_rejected", err != nil)
		err = client.Call(ctx, "messages.thread", map[string]any{"chat_id": fixture.chat, "thread_originator_id": "missing-synthetic-root"}, &invalid)
		add(protocol+"_missing_root_rejected", err != nil)
		var center contracts.ThreadResult
		err = client.Call(ctx, "messages.thread", contracts.ThreadQuery{ChatID: fixture.chat, Anchor: fixture.root, Limit: 100}, &center)
		window := ids(center.Items)
		for _, direction := range []string{"before", "after"} {
			page := center
			for depth := 0; depth < 2; depth++ {
				token := page.Before
				if direction == "after" {
					token = page.After
				}
				if token == "" {
					break
				}
				page = contracts.ThreadResult{}
				if err = client.Call(ctx, "messages.thread", map[string]any{"chat_id": fixture.chat, direction: token, "limit": 100}, &page); err != nil {
					break
				}
				window = append(window, ids(page.Items)...)
			}
		}
		add(protocol+"_replies_outside_current_500_item_window", err == nil && len(window) == 500 && !slices.Contains(window, fixture.replies[0]) && !slices.Contains(window, fixture.replies[204]))
		if budgets {
			batch := []float64{}
			batchValid := true
			query["limit"] = 100
			for sample := 0; sample < 100; sample++ {
				var page contracts.ThreadResult
				start := time.Now()
				err = client.Call(ctx, "messages.thread", query, &page)
				batch = append(batch, float64(time.Since(start))/float64(time.Millisecond))
				batchValid = batchValid && err == nil && len(page.Items) == 100 && page.Items[0].ID == fixture.root
			}
			e.Latency[protocol+"_filtered_batch"] = stats(batch, 0)
			add(protocol+"_filtered_batch_results", batchValid)
			add(protocol+"_filtered_batch_p95_10ms", e.Latency[protocol+"_filtered_batch"].P95 <= 10)
			abEndpoint := w.Gateway
			abEndpoint.Token, err = w.ScopedToken(abEndpoint, "messages:read,contacts:read,system:read")
			add(protocol+"_ab_token", err == nil)
			if err != nil {
				continue
			}
			abClient := scripted.Client{Endpoint: abEndpoint, Protocol: protocol}
			referenceEndpoint := w.Gateway
			referenceEndpoint.Token, err = w.ScopedToken(referenceEndpoint, "messages:read,contacts:read,system:read")
			add(protocol+"_reference_token", err == nil)
			if err != nil {
				continue
			}
			reference := scripted.Client{Endpoint: referenceEndpoint, Protocol: protocol}
			durations, referenceDurations := []float64{}, []float64{}
			valid, referenceValid := true, true
			query["limit"] = 100
			for sample := 0; sample < 100; sample++ {
				for _, filtered := range []bool{sample%2 == 0, sample%2 != 0} {
					var page contracts.ThreadResult
					selected := abClient
					var input any = query
					if !filtered {
						selected = reference
						input = contracts.ThreadQuery{ChatID: fixture.chat, Anchor: fixture.root, Limit: 100}
					}
					start := time.Now()
					err = selected.Call(ctx, "messages.thread", input, &page)
					elapsed := float64(time.Since(start)) / float64(time.Millisecond)
					if filtered {
						durations = append(durations, elapsed)
						valid = valid && err == nil && len(page.Items) == 100 && page.Items[0].ID == fixture.root
					} else {
						referenceDurations = append(referenceDurations, elapsed)
						referenceValid = referenceValid && err == nil && len(page.Items) == 100 && slices.Contains(ids(page.Items), fixture.root)
					}
				}
			}
			e.Latency[protocol+"_unfiltered_reference"] = stats(referenceDurations, 0)
			add(protocol+"_unfiltered_reference_results", referenceValid)
			latency := stats(durations, 0)
			e.Latency[protocol+"_filtered_thread"] = latency
			add(protocol+"_100k_filtered_read_results", valid)
			add(protocol+"_100k_filtered_read_p95_10ms", latency.P95 <= 10)
		}
	}
	denied := w.Gateway
	var err error
	denied.Token, err = w.ScopedToken(w.Gateway, "system:read")
	add("restricted_token", err == nil)
	for _, protocol := range []string{"rest", "mcp"} {
		var page contracts.ThreadResult
		err = (scripted.Client{Endpoint: denied, Protocol: protocol}).Call(ctx, "messages.thread", map[string]any{"chat_id": fixture.chat, "thread_originator_id": fixture.root, "limit": 200}, &page)
		add(protocol+"_filtered_visibility", err == nil && len(page.Items) == 0)
	}
}
