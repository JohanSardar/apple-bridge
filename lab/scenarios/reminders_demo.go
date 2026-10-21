package scenarios

import (
	"context"
	"os/exec"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/demo"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func runRemindersDemo(ctx context.Context, scenario Scenario, level string) Evidence {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	evidence := Evidence{RunID: contracts.NewID("run"), Scenario: scenario.ID, Level: level, GitSHA: strings.TrimSpace(string(sha)), At: time.Now().UTC(), Assertions: []Assertion{}, Latency: map[string]Latency{}, Hosts: []string{"air"}}
	add := func(name string, passed bool) {
		evidence.Assertions = append(evidence.Assertions, Assertion{Name: name, Passed: passed})
	}
	dataset := demo.Generate(demo.Options{Seed: 17})
	observations := []contracts.Observation{}
	for _, observation := range dataset.Records {
		if contracts.CollectionSource(observation.Collection) == contracts.Reminders {
			observations = append(observations, observation)
		}
	}
	add("generated_reminders_demo_records", len(observations) == 16)
	if len(observations) == 0 {
		return evidence
	}
	w, err := world.New(ctx, level, 1, 0, world.BlobOptions{Scope: "reminders:read,system:read"})
	add("demo_reminders_world", err == nil)
	if err != nil {
		return evidence
	}
	defer w.Close()
	err = w.ControlAction("/dataset", map[string]any{"records": observations, "blobs": map[string][]byte{}})
	add("demo_reminders_sim_capture", err == nil)
	if err != nil {
		return evidence
	}
	add("demo_reminders_replica_convergence", w.Await(10*time.Second, func() bool { return convergence(w) }) == nil)
	records, err := w.Records(w.Gateway)
	add("demo_reminders_read_records", err == nil)
	parentIDs := map[string]string{}
	tagsSeen := map[string]bool{}
	groupIDs := map[string]bool{}
	groupParents := []string{}
	ordered, pinned, emblems := 0, 0, 0
	lists, flags, parents, sections, tagged, smart, groups, assigned := 0, 0, 0, 0, 0, 0, 0, 0
	for _, record := range records {
		fields := rmData(record)
		if record.Collection == "reminders.list" {
			lists++
			if fields["is_group"] == true {
				groups++
				groupIDs[record.ID] = true
			}
			if parent, ok := fields["parent_id"].(string); ok && parent != "" {
				groupParents = append(groupParents, parent)
			}
			if _, ok := fields["display_order"].(float64); ok {
				ordered++
			}
			if fields["pinned"] == true {
				pinned++
			}
			if emblem, ok := fields["badge_emblem"].(string); ok && emblem != "" {
				emblems++
			}
			if fields["smart_list_type"] != nil {
				smart++
			}
			if entries, ok := fields["sections"].([]any); ok {
				sections += len(entries)
			}
		}
		if record.Collection != "reminders.reminder" {
			continue
		}
		if fields["flagged"] == true {
			flags++
		}
		if parent, ok := fields["parent_id"].(string); ok && parent != "" {
			parents++
			parentIDs[record.ID] = parent
		}
		if tags, ok := fields["tags"].([]any); ok && len(tags) > 0 {
			tagged++
			for _, tag := range tags {
				if text, ok := tag.(string); ok {
					tagsSeen[text] = true
				}
			}
		}
		if assignment, ok := fields["assignment"].(map[string]any); ok {
			if assignee, ok := assignment["assignee"].(map[string]any); ok && assignee["is_me"] == true {
				if id, ok := assignee["id"].(string); ok && id != "" {
					assigned++
				}
			}
		}
	}
	add("demo_reminders_lists_and_group", lists == 6 && groups == 1)
	linkedGroup := len(groupParents) == 2
	for _, parent := range groupParents {
		linkedGroup = linkedGroup && groupIDs[parent]
	}
	add("demo_reminders_group_reference_resolved", linkedGroup)
	add("demo_reminders_native_order_pin_and_emblems", ordered == 6 && pinned == 1 && emblems == 6)
	add("demo_reminders_flags", flags == 2)
	deep := false
	for _, parent := range parentIDs {
		deep = deep || parentIDs[parent] != ""
	}
	add("demo_reminders_recursive_subtasks", parents == 2 && deep)
	add("demo_reminders_sections", sections == 4)
	add("demo_reminders_tags", tagged == 9 && len(tagsSeen) == 4)
	add("demo_reminders_smart_list", smart == 1)
	add("demo_reminders_source_verified_assignee", assigned == 1)
	checkRemindersDemoReadModels(ctx, w, add)
	evidence.Passed = true
	for _, assertion := range evidence.Assertions {
		evidence.Passed = evidence.Passed && assertion.Passed
	}
	return evidence
}
