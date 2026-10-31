package scenarios

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"os"
	"os/exec"
	"strings"
	"time"
)

func runNotesExtras(ctx context.Context, s Scenario, level string) (e Evidence) {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e = Evidence{GitSHA: strings.TrimSpace(string(sha)), RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, At: time.Now().UTC(), Assertions: []Assertion{}, Metrics: map[string]any{}}
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	defer func() {
		e.Passed = len(e.Assertions) > 0
		for _, a := range e.Assertions {
			e.Passed = e.Passed && a.Passed
		}
	}()
	if level != "sim" {
		return runNotesExtrasReal(ctx, s, level)
	}
	ctx = world.WithOptions(ctx, world.Options{Scope: "notes:read,notes:write,reminders:read,system:read"})
	w, err := world.New(ctx, level, 0, 0)
	if err != nil {
		fmt.Fprintln(os.Stderr, "notes extras world:", err)
		add("world", false)
		return
	}
	defer w.Close()
	client := scripted.Client{Endpoint: w.Gateway, Protocol: "rest"}
	for _, key := range []string{"extras-a", "extras-b"} {
		err = w.ControlAction("/external-change", map[string]any{"key": key, "collection": "notes.folder", "data": map[string]any{"title": "Bridge Test [ab-test extras] " + key, "account": "sim"}})
		if err != nil {
			add("fixture", false)
			return
		}
	}
	err = w.ControlAction("/external-change", map[string]any{"key": "extras-note", "collection": "notes.note", "data": map[string]any{"title": "[ab-test extras] note", "editable": "plain", "locked": false, "plain_text": "synthetic", "pinned": true, "tags": []string{"#bridge-test"}, "blocks": []any{map[string]any{"type": "list_item", "list_style": "checklist", "checked": true, "checklist_id": "ab-test-extras"}}, "folder_id": map[string]any{"$ref": map[string]string{"collection": "notes.folder", "native_key": "extras-a"}}}})
	var note contracts.Record
	var destination string
	err = w.Await(10*time.Second, func() bool {
		rows, _ := w.Records(w.Gateway)
		for _, r := range rows {
			if recordTitle(r) == "[ab-test extras] note" {
				note = r
			}
			if recordTitle(r) == "Bridge Test [ab-test extras] extras-b" {
				destination = r.ID
			}
		}
		return note.ID != "" && destination != ""
	})
	add("tagged_fixture_visible", err == nil)
	if err != nil {
		return
	}
	var fields map[string]any
	json.Unmarshal(note.Data, &fields)
	add("pin_tag_checklist_read", fields["pinned"] == true && len(fields["tags"].([]any)) == 1 && len(fields["blocks"].([]any)) == 1 && fields["blocks"].([]any)[0].(map[string]any)["checked"] == true)
	var folder contracts.Record
	rows, _ := w.Records(w.Gateway)
	for _, r := range rows {
		if r.ID == destination {
			folder = r
		}
	}
	renameArgs, _ := json.Marshal(map[string]any{"id": destination, "title": "Bridge Test [ab-test extras] renamed"})
	var renamed contracts.CommandRecord
	renameErr := client.Call(ctx, "notes.rename_folder", contracts.Command{ID: contracts.NewCommandID(), Args: renameArgs, ExpectedVersion: folder.Version, WaitMS: 15000}, &renamed)
	add("rename_observed", renameErr == nil && renamed.State == "observed")
	add("rename_readback", w.Await(5*time.Second, func() bool {
		rows, _ := w.Records(w.Gateway)
		for _, r := range rows {
			if r.ID == destination {
				return recordTitle(r) == "Bridge Test [ab-test extras] renamed"
			}
		}
		return false
	}) == nil)
	args, _ := json.Marshal(map[string]any{"id": note.ID, "folder_id": destination})
	var out contracts.CommandRecord
	err = client.Call(ctx, "notes.move", contracts.Command{ID: contracts.NewCommandID(), Args: args, ExpectedVersion: note.Version, WaitMS: 15000}, &out)
	add("move_observed", err == nil && out.State == "observed")
	add("move_readback", w.Await(5*time.Second, func() bool {
		rows, _ := w.Records(w.Gateway)
		for _, r := range rows {
			if r.ID == note.ID {
				var d map[string]any
				json.Unmarshal(r.Data, &d)
				return d["folder_id"] == destination
			}
		}
		return false
	}) == nil)
	var stale contracts.CommandRecord
	staleErr := client.Call(ctx, "notes.move", contracts.Command{ID: contracts.NewCommandID(), Args: args, ExpectedVersion: note.Version, WaitMS: 15000}, &stale)
	add("stale_move_rejected", errors.Is(staleErr, contracts.ErrConflict) || stale.Error != nil && stale.Error.Code == "conflict")
	var currentFolder contracts.Record
	rows, _ = w.Records(w.Gateway)
	for _, r := range rows {
		if r.ID == destination {
			currentFolder = r
		}
	}
	untaggedArgs, _ := json.Marshal(map[string]any{"id": destination, "title": "ordinary folder"})
	var rejected contracts.CommandRecord
	rejectedErr := client.Call(ctx, "notes.rename_folder", contracts.Command{ID: contracts.NewCommandID(), Args: untaggedArgs, ExpectedVersion: currentFolder.Version, WaitMS: 15000}, &rejected)
	add("untagged_rename_rejected", errors.Is(rejectedErr, contracts.ErrInvalid) || rejected.Error != nil && rejected.Error.Code == "invalid")
	client.Protocol = "mcp"
	mcpTitle := "Bridge Test [ab-test extras] mcp"
	renameArgs, _ = json.Marshal(map[string]any{"id": destination, "title": mcpTitle})
	var mcpRenamed contracts.CommandRecord
	renameErr = client.Call(ctx, "notes.rename_folder", contracts.Command{ID: contracts.NewCommandID(), Args: renameArgs, ExpectedVersion: currentFolder.Version, WaitMS: 15000}, &mcpRenamed)
	add("mcp_rename_observed", renameErr == nil && mcpRenamed.State == "observed")
	add("mcp_rename_readback", w.Await(5*time.Second, func() bool {
		rows, _ := w.Records(w.Gateway)
		for _, row := range rows {
			if row.ID == destination {
				return recordTitle(row) == mcpTitle
			}
		}
		return false
	}) == nil)
	originalFolder, _ := fields["folder_id"].(string)
	rows, _ = w.Records(w.Gateway)
	for _, row := range rows {
		if row.ID == note.ID {
			note = row
		}
	}
	args, _ = json.Marshal(map[string]any{"id": note.ID, "folder_id": originalFolder})
	var mcpMoved contracts.CommandRecord
	err = client.Call(ctx, "notes.move", contracts.Command{ID: contracts.NewCommandID(), Args: args, ExpectedVersion: note.Version, WaitMS: 15000}, &mcpMoved)
	add("mcp_move_observed", err == nil && mcpMoved.State == "observed")
	add("mcp_move_readback", w.Await(5*time.Second, func() bool {
		rows, _ := w.Records(w.Gateway)
		for _, row := range rows {
			if row.ID == note.ID {
				var data map[string]any
				json.Unmarshal(row.Data, &data)
				return data["folder_id"] == originalFolder
			}
		}
		return false
	}) == nil)
	return
}
