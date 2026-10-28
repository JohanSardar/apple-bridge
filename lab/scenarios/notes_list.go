package scenarios

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func runNotesList(ctx context.Context, s Scenario, level string) (e Evidence) {
	e = Evidence{RunID: contracts.NewID("notes-list"), Scenario: s.ID, Level: level, At: time.Now().UTC(), Assertions: []Assertion{}, Hosts: []string{"air"}}
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	defer func() {
		e.Passed = len(e.Assertions) > 0
		for _, assertion := range e.Assertions {
			e.Passed = e.Passed && assertion.Passed
		}
	}()
	ctx = world.WithOptions(ctx, world.Options{Scope: "notes:read,notes:write,reminders:read,system:read"})
	w, err := world.New(ctx, "sim", 1, 0)
	add("world", err == nil)
	if err != nil {
		fmt.Fprintln(os.Stderr, "notes list world:", err)
		return
	}
	defer w.Close()
	ref := func(key string) any {
		return map[string]any{"$ref": map[string]string{"collection": "notes.folder", "native_key": key}}
	}
	fixtures := []struct {
		collection, key string
		data            map[string]any
	}{
		{"notes.folder", "notes-root", map[string]any{"title": "Bridge Test", "account": "notes-account", "account_name": "Test Account"}},
		{"notes.folder", "notes-child", map[string]any{"title": "Nested", "account": "notes-account", "parent_id": ref("notes-root")}},
		{"notes.folder", "notes-deleted", map[string]any{"title": "Recently Deleted", "account": "notes-account", "recently_deleted": true}},
	}
	for i, title := range []string{"Zulu", "Alpha", "Locked", "Deleted"} {
		folder := "notes-child"
		if i == 3 {
			folder = "notes-deleted"
		}
		data := map[string]any{"title": title, "plain_text": title + "\n[ab-test] Preview 🧪", "folder_id": ref(folder), "created_at": time.Now().UTC().AddDate(0, 0, -10).Format(time.RFC3339), "modified_at": time.Now().UTC().Format(time.RFC3339), "editable": "plain", "content_status": "complete"}
		if i == 0 {
			data["pinned"] = true
			data["attachments"] = []any{map[string]any{"derivatives": map[string]any{"thumb": map[string]any{"digest": strings.Repeat("b", 64), "state": "present"}}}}
		}
		if i == 1 {
			data["plain_text"] = ""
			data["preview"] = "[ab-test] Preview 🧪"
			data["content_status"] = "offloaded"
			data["editable"] = "none"
		}
		if i == 2 {
			data["locked"] = true
			data["html"] = "private locked body"
			data["blocks"] = []any{"private locked body"}
			data["thumbnail"] = map[string]any{"digest": strings.Repeat("c", 64)}
		}
		fixtures = append(fixtures, struct {
			collection, key string
			data            map[string]any
		}{"notes.note", fmt.Sprintf("notes-list-%d", i), data})
	}
	for _, fixture := range fixtures {
		if err = w.ControlAction("/external-change", map[string]any{"collection": fixture.collection, "key": fixture.key, "data": fixture.data}); err != nil {
			break
		}
	}
	add("captured", err == nil && w.FlushCaptures(10*time.Second) == nil)
	add("replica_converged", w.Await(10*time.Second, func() bool { return convergence(w) }) == nil)
	if !e.Assertions[len(e.Assertions)-1].Passed {
		e.Assertions[len(e.Assertions)-1].Detail = convergenceFailure(w)
	}
	aliases, _ := gatewayAliases(w)
	child := aliases[aliasKey(contracts.Notes, "notes.folder", "notes-child")]
	deleted := aliases[aliasKey(contracts.Notes, "notes.folder", "notes-deleted")]
	for index, endpoint := range append([]world.Endpoint{w.Gateway}, w.Replicas...) {
		records, err := w.Records(endpoint)
		validVersions, noteCount := err == nil, 0
		for _, record := range records {
			if record.Collection != "notes.note" {
				continue
			}
			noteCount++
			var content struct {
				Version uint64 `json:"content_version"`
			}
			validVersions = validVersions && json.Unmarshal(record.Data, &content) == nil && content.Version > 0 && content.Version <= record.Version
		}
		add(fmt.Sprintf("content_versions_%d", index), validVersions && noteCount == 4)
		for _, protocol := range []string{"rest", "mcp"} {
			client := scripted.Client{Endpoint: endpoint, Protocol: protocol}
			prefix := fmt.Sprintf("%s_%d_", protocol, index)
			var out contracts.NotesListResult
			err = client.Call(ctx, "notes.list", contracts.NotesListQuery{Folder: child, Sort: "modified", TimeZone: "UTC"}, &out)
			add(prefix+"pinned_date_sections", err == nil && out.Count == 3 && len(out.Sections) == 2 && out.Sections[0].Key == "pinned" && out.Sections[1].Key == "today")
			locked, preview, thumbnail, nested := false, false, false, false
			for _, section := range out.Sections {
				for _, item := range section.Notes {
					var fields map[string]any
					json.Unmarshal(item.Note.Data, &fields)
					if fields["title"] == "Locked" {
						locked = item.ReadOnly && item.Preview == "" && item.Thumbnail == nil && fields["plain_text"] == nil && fields["html"] == nil && fields["blocks"] == nil && fields["thumbnail"] == nil
					}
					if fields["title"] == "Zulu" {
						preview = item.Preview == "[ab-test] Preview 🧪"
						thumbnail = item.Thumbnail != nil && item.Thumbnail.Digest == strings.Repeat("b", 64)
					}
				}
			}
			for _, folder := range out.Folders {
				if folder.Folder.ID == child {
					nested = folder.ParentID != nil && folder.Count == 3 && folder.AccountID == "notes-account"
				}
			}
			add(prefix+"locked_title_only", locked)
			add(prefix+"preview_thumbnail", preview && thumbnail)
			add(prefix+"account_nested_counts", nested && len(out.Accounts) == 1 && out.Accounts[0].Count == 3)
			err = client.Call(ctx, "notes.list", contracts.NotesListQuery{Folder: child, Sort: "title"}, &out)
			add(prefix+"title_sort", err == nil && len(out.Sections) == 2 && len(out.Sections[1].Notes) == 2 && strings.Contains(string(out.Sections[1].Notes[0].Note.Data), "Alpha"))
			err = client.Call(ctx, "notes.list", contracts.NotesListQuery{Folder: child, Sort: "created"}, &out)
			add(prefix+"created_date_groups", err == nil && len(out.Sections) == 2 && out.Sections[1].Key == "previous_30_days")
			err = client.Call(ctx, "notes.list", contracts.NotesListQuery{Folder: deleted}, &out)
			add(prefix+"recently_deleted_read_only", err == nil && out.Count == 1 && len(out.Sections) == 1 && out.Sections[0].Notes[0].ReadOnly)
			if err == nil && out.Count == 1 {
				note := out.Sections[0].Notes[0].Note
				var command contracts.CommandRecord
				err = client.Call(ctx, "notes.delete", map[string]any{"command_id": contracts.NewCommandID(), "expected_version": note.Version, "wait_ms": 15000, "args": map[string]any{"id": note.ID}}, &command)
				effects, effectErr := w.Effects()
				add(prefix+"deleted_folder_write_refused", (errors.Is(err, contracts.ErrUnsupported) || (command.State == "rejected" && command.Error != nil && command.Error.Code == "unsupported")) && effectErr == nil && len(effects) == 0)
			}
			err = client.Call(ctx, "notes.list", contracts.NotesListQuery{}, &out)
			add(prefix+"all_excludes_deleted", err == nil && out.Count == 3)
			add(prefix+"invalid_sort_refused", client.Call(ctx, "notes.list", map[string]any{"sort": "invalid"}, &out) != nil)
			add(prefix+"invalid_zone_refused", client.Call(ctx, "notes.list", map[string]any{"time_zone": "Invalid/Zone"}, &out) != nil)
			restricted := endpoint
			restricted.Token, _ = w.ScopedToken(endpoint, "system:read")
			err = (scripted.Client{Endpoint: restricted, Protocol: protocol}).Call(ctx, "notes.list", contracts.NotesListQuery{}, &out)
			add(prefix+"notes_scope", err == nil && out.Count == 0 && len(out.Accounts) == 0 && len(out.Folders) == 0)
		}
	}
	return
}
