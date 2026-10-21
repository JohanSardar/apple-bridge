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

func runRemindersItems(ctx context.Context, s Scenario, level string) Evidence {
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
		var result map[string]any
		err = client.Call(ctx, "reminders.items", map[string]any{"time_zone": "America/New_York", "list": listID}, &result)
		add(protocol+"_reminders.items_available", err == nil)
		if err != nil {
			available = false
			continue
		}
		type item struct {
			Reminder contracts.Record `json:"reminder"`
			Subtasks []struct {
				Reminder contracts.Record `json:"reminder"`
			} `json:"subtasks"`
		}
		var page struct {
			Count    int `json:"count"`
			Sections []struct {
				Section struct{ Kind, ID, Title string } `json:"section"`
				Items   []item                           `json:"items"`
			} `json:"sections"`
		}
		raw, _ := json.Marshal(result)
		add(protocol+"_section_shape_and_filtered_count", json.Unmarshal(raw, &page) == nil && page.Count == 2 && len(page.Sections) == 2)
		if len(page.Sections) == 2 {
			add(protocol+"_native_and_empty_section_order", page.Sections[0].Section.Kind == "store" && page.Sections[0].Section.ID == "section-a" && page.Sections[1].Section.ID == "section-empty" && len(page.Sections[1].Items) == 0)
			roots := page.Sections[0].Items
			add(protocol+"_subtasks_nested_once", len(roots) == 1 && recordTitle(roots[0].Reminder) == "Bridge Test Parent" && len(roots[0].Subtasks) == 1 && recordTitle(roots[0].Subtasks[0].Reminder) == "Bridge Test Child")
		}
	}
	if available {
		reminderReadModelProtocolParity(ctx, w, "reminders.items", map[string]any{"time_zone": "America/New_York", "list": listID}, add)
		reminderReadModelSchema(ctx, w, "reminders.items", add)
		reminderReadModelGroups(ctx, w, listID, add)
		reminderItemsReplica(ctx, w, listID, add)
		reminderReadModelZones(ctx, w, add)
		reminderItemsBoundaries(ctx, w, listID, add)
		reminderReadModelAttachments(ctx, w, listID, add)
		reminderReadModelNesting(ctx, w, listID, add)
		reminderReadModelSorts(ctx, w, add)
		reminderReadModelPerf(ctx, &e, add)
	}
	e.Passed = true
	for _, assertion := range e.Assertions {
		e.Passed = e.Passed && assertion.Passed
	}
	return e
}
