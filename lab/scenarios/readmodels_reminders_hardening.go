package scenarios

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"os/exec"
	"reflect"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func runRemindersHardening(ctx context.Context, scenario Scenario, level string) Evidence {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e := Evidence{RunID: contracts.NewID("run"), Scenario: scenario.ID, Level: level, GitSHA: strings.TrimSpace(string(sha)), At: time.Now().UTC(), Assertions: []Assertion{}, Latency: map[string]Latency{}, Hosts: []string{"air"}}
	add := func(name string, passed bool) {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: passed})
	}
	reminderNaturalTitles(ctx, level, add)
	reminderLargeSelection(ctx, level, add)
	e.Passed = true
	for _, assertion := range e.Assertions {
		e.Passed = e.Passed && assertion.Passed
	}
	return e
}

func reminderNaturalTitles(ctx context.Context, level string, add func(string, bool)) {
	w, err := world.New(ctx, level, 0, 0, world.BlobOptions{Scope: "reminders:read,messages:read,calendar:read,system:read"})
	add("natural_title_world", err == nil)
	if err != nil {
		return
	}
	defer w.Close()
	add("natural_title_list", w.ControlAction("/external-change", map[string]any{"collection": "reminders.list", "key": "rm-natural-list", "data": map[string]any{"title": "Bridge Test Numeric Titles"}}) == nil)
	for index, title := range []string{"Item 10", "Item 2", "item 1", "Item 12"} {
		data := map[string]any{"title": title, "list_id": rmRef("reminders.list", "rm-natural-list"), "completed": false}
		add("natural_title_fixture", w.ControlAction("/external-change", map[string]any{"collection": "reminders.reminder", "key": fmt.Sprintf("rm-natural-%d", index), "data": data}) == nil)
	}
	add("natural_title_convergence", w.Await(10*time.Second, func() bool { return convergence(w) }) == nil)
	records, err := w.Records(w.Gateway)
	listID := ""
	for _, record := range records {
		if record.Collection == "reminders.list" && recordTitle(record) == "Bridge Test Numeric Titles" {
			listID = record.ID
		}
	}
	add("natural_title_list_resolved", err == nil && listID != "")
	for _, protocol := range []string{"rest", "mcp"} {
		var page contracts.ReminderItemsResult
		client := scripted.Client{Endpoint: w.Gateway, Protocol: protocol}
		err = client.Call(ctx, "reminders.items", contracts.ReminderItemsQuery{List: listID, TimeZone: "America/New_York", Sort: "title"}, &page)
		titles := []string{}
		for _, section := range page.Sections {
			for _, item := range section.Items {
				titles = append(titles, recordTitle(item.Reminder))
			}
		}
		add(protocol+"_numeric_case_insensitive_title_order", err == nil && reflect.DeepEqual(titles, []string{"item 1", "Item 2", "Item 10", "Item 12"}))
		for _, operation := range []string{"messages.chats", "sources.status"} {
			var headers http.Header
			var body []byte
			client.Observe = func(h http.Header, b []byte) { headers, body = h, b }
			err = client.Call(ctx, operation, map[string]any{}, nil)
			_, _, measured := reminderHandlerTiming(protocol, headers, body)
			add(protocol+"_descriptor_timing_"+operation, err == nil && measured == (operation == "messages.chats"))
		}
	}
	reminderOtherProjectionParity(ctx, w, add)
}

func reminderLargeSelection(ctx context.Context, level string, add func(string, bool)) {
	const count = 33000
	w, err := world.New(ctx, level, 0, count, world.BlobOptions{Scope: "reminders:read,system:read"})
	add("large_selection_world", err == nil)
	if err != nil {
		return
	}
	defer w.Close()
	add("large_selection_convergence", w.Await(60*time.Second, func() bool { return convergence(w) }) == nil)
	endpoint := w.Gateway
	endpoint.Token, err = w.ScopedToken(w.Gateway, "reminders:read,system:read")
	add("large_selection_read_token", err == nil)
	if err != nil {
		return
	}
	records, err := w.Records(endpoint)
	listID := ""
	ids := []string{}
	for _, record := range records {
		if record.Collection == "reminders.list" {
			listID = record.ID
		}
		if record.Collection == "reminders.reminder" {
			ids = append(ids, record.ID)
		}
	}
	add("large_selection_above_sqlite_variable_limit", err == nil && len(ids) == count && listID != "")
	if len(ids) != count || listID == "" {
		return
	}
	owners := []string{ids[0], ids[len(ids)-1]}
	for index, owner := range owners {
		data := map[string]any{"reminder_id": owner, "filename": fmt.Sprintf("Bridge Test Boundary %d.png", index), "uti": "public.png", "size": 0, "media": nil}
		add("large_selection_attachment", w.ControlAction("/external-change", map[string]any{"collection": "reminders.attachment", "key": fmt.Sprintf("rm-boundary-attachment-%d", index), "data": data}) == nil)
	}
	add("large_selection_attachment_convergence", w.Await(60*time.Second, func() bool { return convergence(w) }) == nil)
	var page contracts.ReminderItemsResult
	endpoint.Token, err = w.ScopedToken(w.Gateway, "reminders:read,system:read")
	add("large_selection_query_token", err == nil)
	if err != nil {
		return
	}
	err = (scripted.Client{Endpoint: endpoint}).Call(ctx, "reminders.items", contracts.ReminderItemsQuery{List: listID, TimeZone: "America/New_York"}, &page)
	attachments := map[string]string{}
	items := 0
	for _, section := range page.Sections {
		for _, item := range section.Items {
			items++
			for _, attachment := range item.Attachments {
				var data struct {
					ReminderID string `json:"reminder_id"`
					Filename   string `json:"filename"`
				}
				json.Unmarshal(attachment.Data, &data)
				if data.ReminderID == item.Reminder.ID {
					attachments[data.ReminderID] = data.Filename
				}
			}
		}
	}
	add("rest_large_selection_and_attachment_owners_exact", err == nil && page.Count == count && items == count && len(attachments) == 2 && attachments[owners[0]] == "Bridge Test Boundary 0.png" && attachments[owners[1]] == "Bridge Test Boundary 1.png")
}

func reminderOtherProjectionParity(ctx context.Context, w *world.World, add func(string, bool)) {
	at := time.Now().UTC().Truncate(time.Minute).Add(time.Hour)
	add("timed_messages_fixture", rmPut(w,
		rmObservation("messages.chat", "rm-timing-chat", map[string]any{"service": "iMessage", "display_name": "Bridge Test Timing", "handles": []string{}, "is_group": false}),
		rmObservation("messages.message", "rm-timing-message", map[string]any{"chat_id": rmRef("messages.chat", "rm-timing-chat"), "sent_at": at, "text": "[ab-test] timing parity", "kind": "message", "is_from_me": true, "parts": []any{}}),
	) == nil)
	add("timed_calendar_fixture", w.ControlAction("/calendar", map[string]any{"action": "calendar", "key": "test-calendar"}) == nil)
	add("timed_event_fixture", w.ControlAction("/calendar", map[string]any{"action": "create", "key": "rm-timing-event", "data": map[string]any{"title": "Bridge Test Timing", "start": at.Format(time.RFC3339), "end": at.Add(30 * time.Minute).Format(time.RFC3339)}}) == nil)
	add("timed_projection_convergence", w.Await(10*time.Second, func() bool { return convergence(w) }) == nil)
	for operation, query := range map[string]map[string]any{
		"messages.chats":  {},
		"calendar.agenda": {"start": at.Add(-time.Hour).Format(time.RFC3339), "end": at.Add(time.Hour).Format(time.RFC3339)},
	} {
		var response map[string]any
		err := (scripted.Client{Endpoint: w.Gateway}).Call(ctx, operation, query, &response)
		key := "records"
		if operation == "messages.chats" {
			key = "chats"
		}
		records, _ := response[key].([]any)
		add(operation+"_nonempty_timed_projection", err == nil && len(records) == 1)
		reminderReadModelProtocolParity(ctx, w, operation, query, add)
	}
}
