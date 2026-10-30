package scenarios

import (
	"context"
	"encoding/json"
	"errors"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func runNotesExtrasReal(ctx context.Context, s Scenario, level string) (e Evidence) {
	e = Evidence{RunID: contracts.NewID("notes-extras"), Scenario: s.ID, Level: level, At: time.Now().UTC(), Metrics: map[string]any{}}
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	defer func() {
		e.Passed = len(e.Assertions) > 0
		for _, a := range e.Assertions {
			e.Passed = e.Passed && a.Passed
		}
	}()
	before, err := cleanupSnapshot(ctx, "notes")
	if err != nil {
		add("cleanup_baseline", false)
		return
	}
	audit, err := newRunCleanup(e.RunID, before)
	if err != nil {
		add("cleanup_ledger", false)
		return
	}
	defer func() { finishCleanupAudit(context.Background(), audit, "notes", &e) }()
	ctx = world.WithOptions(ctx, world.Options{Scope: "notes:read,notes:write,system:read"})
	w, err := world.NewReal(ctx, level, 0)
	if err != nil {
		add("world", false)
		return
	}
	defer w.Close()
	tag := "[ab-test " + e.RunID + "]"
	title := "Bridge Test " + tag
	folders := map[string]string{}
	noteID := ""
	defer func() {
		if noteID != "" {
			_, problem := w.RecoverTruth("fixture", map[string]any{"source": "notes", "tag": tag, "action": "cleanup_owned", "id": noteID, "folder_id": func() string {
				observed, _ := w.RecoverTruth("observe", map[string]any{"source": "notes", "tag": tag})
				rows, _ := observed["records"].([]any)
				for _, raw := range rows {
					row := raw.(map[string]any)
					if row["id"] == noteID {
						id, _ := row["folder_id"].(string)
						return id
					}
				}
				return ""
			}()})
			add("owned_note_cleaned", problem == nil)
		}
		for id, name := range folders {
			_, problem := w.RecoverTruth("fixture", map[string]any{"source": "notes", "tag": tag, "action": "cleanup_created_folder", "created_by_run": true, "folder_id": id, "title": name})
			add("owned_folder_cleaned", problem == nil)
		}
	}()
	client := scripted.Client{Endpoint: w.Gateway, Protocol: "rest"}
	call := func(op string, args map[string]any, version uint64) bool {
		bytes, _ := json.Marshal(args)
		var out contracts.CommandRecord
		problem := client.Call(ctx, op, contracts.Command{ID: contracts.NewCommandID(), Args: bytes, ExpectedVersion: version, WaitMS: 15000}, &out)
		e.Assertions = append(e.Assertions, Assertion{Name: strings.TrimPrefix(op, "notes.") + "_observed", Passed: problem == nil && out.State == "observed", Detail: string(out.State)})
		return problem == nil && out.State == "observed"
	}
	find := func(name string) (r contracts.Record) {
		w.Await(10*time.Second, func() bool {
			rows, _ := w.Records(w.Gateway)
			for _, row := range rows {
				if !row.Deleted && recordTitle(row) == name {
					r = row
					return true
				}
			}
			return false
		})
		return
	}
	source, problem := w.Truth("fixture", map[string]any{"source": "notes", "tag": tag, "action": "create", "title": tag + " note", "folder_title": title})
	add("tagged_fixture_created", problem == nil)
	if problem != nil {
		return
	}
	noteID, _ = source["id"].(string)
	folderID, _ := source["folder_id"].(string)
	if source["folder_created"] == true {
		folders[folderID] = title
	}
	note := find(tag + " note")
	add("tagged_fixture_visible", note.ID != "")
	if note.ID == "" {
		return
	}
	destinationTitle := title + " destination"
	createdDestination := call("notes.create_folder", map[string]any{"title": destinationTitle}, 0)
	identity, problem := w.Truth("fixture", map[string]any{"source": "notes", "tag": tag, "action": "folder_identity", "title": destinationTitle})
	destinationID, _ := identity["id"].(string)
	add("destination_identity_captured", problem == nil && destinationID != "")
	if destinationID == "" {
		return
	}
	folders[destinationID] = destinationTitle
	if !createdDestination {
		return
	}
	destination := find(destinationTitle)
	if destination.ID == "" {
		add("destination_visible", false)
		return
	}
	renamedTitle := title + " renamed"
	if !call("notes.rename_folder", map[string]any{"id": destination.ID, "title": renamedTitle}, destination.Version) {
		return
	}
	folders[destinationID] = renamedTitle
	add("rename_readback", find(renamedTitle).ID == destination.ID)
	if !call("notes.move", map[string]any{"id": note.ID, "folder_id": destination.ID}, note.Version) {
		return
	}
	add("move_readback", w.Await(10*time.Second, func() bool {
		rows, _ := w.Records(w.Gateway)
		for _, row := range rows {
			if row.ID == note.ID {
				var data map[string]any
				json.Unmarshal(row.Data, &data)
				return data["folder_id"] == destination.ID
			}
		}
		return false
	}) == nil)
	staleArgs, _ := json.Marshal(map[string]any{"id": note.ID, "folder_id": destination.ID})
	var stale contracts.CommandRecord
	staleErr := client.Call(ctx, "notes.move", contracts.Command{ID: contracts.NewCommandID(), Args: staleArgs, ExpectedVersion: note.Version, WaitMS: 15000}, &stale)
	add("stale_move_rejected", errors.Is(staleErr, contracts.ErrConflict) || stale.Error != nil && stale.Error.Code == "conflict")
	observed, problem := w.Truth("observe", map[string]any{"source": "notes", "tag": tag})
	matched := false
	rows, _ := observed["records"].([]any)
	for _, raw := range rows {
		row := raw.(map[string]any)
		matched = matched || (row["id"] == noteID && row["folder_id"] == destinationID)
	}
	add("independent_move_readback", problem == nil && matched)
	return
}
