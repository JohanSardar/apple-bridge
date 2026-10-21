package scenarios

import (
	"context"
	"encoding/json"
	"os/exec"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func runRemindersSidebar(ctx context.Context, s Scenario, level string) Evidence {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e := Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, GitSHA: strings.TrimSpace(string(sha)), At: time.Now().UTC(), Assertions: []Assertion{}, Latency: map[string]Latency{}, Hosts: []string{"air"}}
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	w, err := world.New(ctx, level, 1, 0, world.BlobOptions{Scope: "reminders:read,system:read"})
	add("world_start", err == nil)
	if err != nil {
		return e
	}
	defer w.Close()
	fixtures := reminderReadModelFixtures()
	for _, fixture := range fixtures {
		err = w.ControlAction("/external-change", map[string]any{"collection": fixture.Collection, "key": fixture.NativeKey, "data": fixture.Data})
		if err != nil {
			add("synthetic_reminders_capture", false)
			return e
		}
	}
	add("synthetic_reminders_gateway_and_replica", w.Await(5*time.Second, func() bool { return convergence(w) }) == nil)
	records, err := w.Records(w.Gateway)
	listID := ""
	for _, record := range records {
		if record.Collection == "reminders.list" && recordTitle(record) == "Bridge Test Read Models" {
			listID = record.ID
		}
	}
	add("list_reference_resolved", err == nil && listID != "")
	available := true
	for _, protocol := range []string{"rest", "mcp"} {
		client := scripted.Client{Endpoint: w.Gateway, Protocol: protocol}
		query := map[string]any{"time_zone": "America/New_York"}
		operation := "reminders.lists"
		var result map[string]any
		err = client.Call(ctx, operation, query, &result)
		add(protocol+"_"+operation+"_available", err == nil)
		if err != nil {
			available = false
			continue
		}
		if operation == "reminders.lists" {
			var page struct {
				Smart []struct {
					ID    string `json:"id"`
					Count *int   `json:"count"`
				} `json:"smart"`
				Lists []struct {
					List    contracts.Record `json:"list"`
					GroupID *string          `json:"group_id"`
					Count   *int             `json:"count"`
					Pinned  bool             `json:"pinned"`
				} `json:"lists"`
				Tags []struct {
					Name  string `json:"name"`
					Count int    `json:"count"`
				} `json:"tags"`
			}
			raw, _ := json.Marshal(result)
			add(protocol+"_sidebar_shape", json.Unmarshal(raw, &page) == nil && len(page.Lists) == 4)
			for _, tile := range page.Smart {
				expected, found := map[string]int{"today": 2, "scheduled": 3, "all": 4, "flagged": 1, "completed": 1, "assigned": 1}[tile.ID]
				add(protocol+"_smart_"+tile.ID+"_definition", found && tile.Count != nil && *tile.Count == expected)
			}
			add(protocol+"_six_smart_tiles", len(page.Smart) == 6)
			if len(page.Lists) == 4 {
				add(protocol+"_native_sidebar_order", recordTitle(page.Lists[0].List) == "Bridge Test Pinned" && page.Lists[0].Pinned && recordTitle(page.Lists[1].List) == "Bridge Test Group" && page.Lists[2].List.ID == listID && recordTitle(page.Lists[3].List) == "Bridge Test Other")
				add(protocol+"_flat_group_and_incomplete_count", page.Lists[2].GroupID != nil && *page.Lists[2].GroupID == page.Lists[1].List.ID && page.Lists[2].Count != nil && *page.Lists[2].Count == 2)
			}
			counts := map[string]int{}
			for _, tag := range page.Tags {
				counts[tag.Name] = tag.Count
			}
			add(protocol+"_incomplete_tag_counts", len(counts) == 3 && counts["work"] == 2 && counts["home"] == 1 && counts["errands"] == 1)
		}
	}
	if available {
		reminderReadModelProtocolParity(ctx, w, "reminders.lists", map[string]any{"time_zone": "America/New_York"}, add)
		reminderReadModelSchema(ctx, w, "reminders.lists", add)
		reminderReadModelReplica(ctx, w, add)
		reminderReadModelBoundaries(ctx, w, add)
	}
	e.Passed = true
	for _, assertion := range e.Assertions {
		e.Passed = e.Passed && assertion.Passed
	}
	return e
}
