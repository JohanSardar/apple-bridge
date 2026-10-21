package scenarios

import (
	"context"
	"encoding/json"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"

	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func reminderItemsBoundaries(ctx context.Context, w *world.World, listID string, add func(string, bool)) {
	denied := w.Gateway
	var err error
	denied.Token, err = w.ScopedToken(w.Gateway, "system:read")
	add("restricted_token", err == nil)
	for _, protocol := range []string{"rest", "mcp"} {
		client := scripted.Client{Endpoint: w.Gateway, Protocol: protocol}
		for _, query := range []map[string]any{
			{"list": listID},
			{"list": listID, "time_zone": "not/a/zone"},
			{"time_zone": "America/New_York"},
			{"list": listID, "smart": "all", "time_zone": "America/New_York"},
			{"smart": "invented", "time_zone": "America/New_York"},
			{"list": listID, "sort": "invented", "time_zone": "America/New_York"},
		} {
			var result map[string]any
			add(protocol+"_invalid_selector_zone_or_sort", client.Call(ctx, "reminders.items", query, &result) != nil)
		}
		client.Endpoint = denied
		var sidebar struct {
			Lists, Smart, Tags []any
		}
		err = client.Call(ctx, "reminders.lists", map[string]any{"time_zone": "America/New_York"}, &sidebar)
		add(protocol+"_visibility_before_sidebar_counts", err == nil && len(sidebar.Lists) == 0 && len(sidebar.Smart) == 0 && len(sidebar.Tags) == 0)
		var items struct {
			Count    int
			Sections []any
		}
		err = client.Call(ctx, "reminders.items", map[string]any{"list": listID, "time_zone": "America/New_York"}, &items)
		add(protocol+"_visibility_before_items_and_nesting", err == nil && items.Count == 0 && len(items.Sections) == 0)
	}
	fixture := reminderReadModelFixtures()[4]
	var data map[string]any
	json.Unmarshal(fixture.Data, &data)
	data["assignment"] = map[string]any{"assignee": map[string]any{"id": "fixture-self", "is_me": nil}}
	add("unknown_assignment_identity_capture", w.ControlAction("/external-change", map[string]any{"collection": fixture.Collection, "key": fixture.NativeKey, "data": data}) == nil)
	add("unknown_assignment_identity_converged", w.Await(5*time.Second, func() bool { return convergence(w) }) == nil)
	add("unknown_custom_filter_capture", w.ControlAction("/external-change", map[string]any{"collection": "reminders.list", "key": "rm-rem-custom", "data": map[string]any{"title": "Bridge Test Unsupported Smart", "smart_list_type": "custom", "smart_filter": nil, "store_coverage": map[string]any{"smart_filter": map[string]any{"status": "unsupported"}}}}) == nil)
	add("unknown_custom_filter_converged", w.Await(5*time.Second, func() bool { return convergence(w) }) == nil)
	for _, protocol := range []string{"rest", "mcp"} {
		client := scripted.Client{Endpoint: w.Gateway, Protocol: protocol}
		var page struct {
			Lists []struct {
				List   contracts.Record
				Count  json.RawMessage
				Status string
			}
			Smart []struct {
				ID     string
				Count  json.RawMessage
				Status string
			}
		}
		err = client.Call(ctx, "reminders.lists", map[string]any{"time_zone": "America/New_York"}, &page)
		unsupported := false
		for _, tile := range page.Smart {
			if tile.ID == "assigned" {
				unsupported = tile.Status == "unsupported" && len(tile.Count) == 0
			}
		}
		add(protocol+"_assigned_identity_unknown_has_no_count", err == nil && unsupported)
		customUnsupported := false
		for _, list := range page.Lists {
			if recordTitle(list.List) == "Bridge Test Unsupported Smart" {
				customUnsupported = list.Status == "unsupported" && len(list.Count) == 0
			}
		}
		add(protocol+"_custom_filter_unknown_has_no_count", err == nil && customUnsupported)
	}
}
