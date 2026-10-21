package scenarios

import (
	"context"
	"encoding/json"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"

	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func reminderReadModelBoundaries(ctx context.Context, w *world.World, add func(string, bool)) {
	denied := w.Gateway
	var err error
	denied.Token, err = w.ScopedToken(w.Gateway, "system:read")
	add("restricted_token", err == nil)
	for _, protocol := range []string{"rest", "mcp"} {
		client := scripted.Client{Endpoint: w.Gateway, Protocol: protocol}
		for _, zone := range []string{"", "Local", "not/a/zone"} {
			var result map[string]any
			add(protocol+"_invalid_viewer_zone", client.Call(ctx, "reminders.lists", map[string]any{"time_zone": zone}, &result) != nil)
		}
		client.Endpoint = denied
		var sidebar struct {
			Lists, Smart, Tags []any
		}
		err = client.Call(ctx, "reminders.lists", map[string]any{"time_zone": "America/New_York"}, &sidebar)
		add(protocol+"_visibility_before_sidebar_counts", err == nil && len(sidebar.Lists) == 0 && len(sidebar.Smart) == 0 && len(sidebar.Tags) == 0)

	}
	fixture := reminderReadModelFixtures()[4]
	var data map[string]any
	json.Unmarshal(fixture.Data, &data)
	data["tags"] = nil
	data["flagged"] = nil
	data["assignment"] = map[string]any{"assignee": map[string]any{"id": "fixture-self", "is_me": nil}}
	add("unknown_assignment_identity_capture", w.ControlAction("/external-change", map[string]any{"collection": fixture.Collection, "key": fixture.NativeKey, "data": data}) == nil)
	add("unknown_assignment_identity_converged", w.Await(5*time.Second, func() bool { return convergence(w) }) == nil)
	add("unknown_custom_filter_capture", w.ControlAction("/external-change", map[string]any{"collection": "reminders.list", "key": "rm-rem-custom", "data": map[string]any{"title": "Bridge Test Unsupported Smart", "smart_list_type": "custom", "smart_filter": nil, "store_coverage": map[string]any{"smart_filter": map[string]any{"status": "unsupported"}}}}) == nil)
	add("unknown_custom_filter_converged", w.Await(5*time.Second, func() bool { return convergence(w) }) == nil)
	for _, protocol := range []string{"rest", "mcp"} {
		client := scripted.Client{Endpoint: w.Gateway, Protocol: protocol}
		var page struct {
			Tags  []any
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
		unsupported, flagUnsupported := false, false
		for _, tile := range page.Smart {
			if tile.ID == "flagged" {
				flagUnsupported = tile.Status == "unsupported" && len(tile.Count) == 0
			}
			if tile.ID == "assigned" {
				unsupported = tile.Status == "unsupported" && len(tile.Count) == 0
			}
		}
		add(protocol+"_assigned_identity_unknown_has_no_count", err == nil && unsupported)
		add(protocol+"_flag_unknown_has_no_count", err == nil && flagUnsupported)
		add(protocol+"_unknown_tags_hide_partial_counts", err == nil && len(page.Tags) == 0)
		customUnsupported := false
		for _, list := range page.Lists {
			if recordTitle(list.List) == "Bridge Test Unsupported Smart" {
				customUnsupported = list.Status == "unsupported" && len(list.Count) == 0
			}
		}
		add(protocol+"_custom_filter_unknown_has_no_count", err == nil && customUnsupported)
	}
}
