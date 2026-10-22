package scenarios

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func runRemindersWriteReal(ctx context.Context, s Scenario, level string) (result Evidence) {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e := Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, GitSHA: strings.TrimSpace(string(sha)), At: time.Now().UTC(), Hosts: []string{"air"}, Assertions: []Assertion{}, Metrics: map[string]any{"browser_proof": "WEB-REM-WRITE web; this level independently verifies the same commands through EventKit"}}
	add := func(name string, passed bool) {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: passed})
	}
	number := func(value any, expected string) bool {
		n, ok := value.(json.Number)
		return ok && n.String() == expected
	}
	tag := "[ab-test " + e.RunID + "]"
	listName := "Bridge Test " + tag
	destinationTag := "[ab-test " + e.RunID + "-destination]"
	destinationName := "Bridge Test " + destinationTag
	prefix := tag + " " + destinationTag
	owners := map[string]map[string]string{}
	owned := []string{}
	requests := []map[string]any{}
	worldDir := ""
	cleaned := false
	recovery := filepath.Join(os.Getenv("HOME"), "Library", "Application Support", "AppleBridgeDev", "w5-write-"+e.RunID+"-recovery.json")
	journal := func() error {
		if err := os.MkdirAll(filepath.Dir(recovery), 0700); err != nil {
			return err
		}
		data, err := json.Marshal(map[string]any{"list": listName, "tag": tag, "created_list_ids": owned, "owners": owners, "requests": requests, "world_dir": worldDir})
		if err != nil {
			return err
		}
		return os.WriteFile(recovery, data, 0600)
	}
	setup := world.NativeSetup{
		BeforeStart: func(w *world.World) error {
			worldDir = w.Dir
			mediaHome := filepath.Join(w.Dir, "gateway", "helper")
			if err := os.MkdirAll(mediaHome, 0700); err != nil {
				return err
			}
			if err := os.WriteFile(filepath.Join(mediaHome, "lab-pause-media"), []byte("paused"), 0600); err != nil {
				return err
			}
			created, err := w.Truth("fixture", map[string]any{"source": "reminders", "action": "create_list", "list": listName, "tag": tag})
			if err != nil {
				return err
			}
			id, _ := created["id"].(string)
			if id == "" || created["created"] != true {
				return fmt.Errorf("unique_fixture_list_not_created")
			}
			owned = append(owned, id)
			owners[id] = map[string]string{"list": listName, "tag": tag}
			return journal()
		},
		Cleanup: func(w *world.World) error {
			if len(owned) == 0 {
				cleaned = true
				return nil
			}
			for _, id := range owned {
				request := map[string]any{"source": "reminders", "action": "cleanup", "list": owners[id]["list"], "list_id": id, "tag": owners[id]["tag"]}
				if _, err := w.RecoverTruth("fixture", request); err != nil {
					return err
				}
				request["action"] = "cleanup_list"
				if _, err := w.RecoverTruth("fixture", request); err != nil {
					return err
				}
				sweep, err := w.RecoverTruth("observe", map[string]any{"source": "reminders", "list": owners[id]["list"], "list_id": id, "tag": owners[id]["tag"], "all_lists": true})
				if err != nil || sweep["list_exists"] != false {
					return fmt.Errorf("owned_fixture_cleanup_sweep_failed")
				}
			}
			for _, scope := range []map[string]string{{"list": listName, "tag": tag}, {"list": destinationName, "tag": destinationTag}} {
				sweep, err := w.RecoverTruth("observe", map[string]any{"source": "reminders", "list": scope["list"], "tag": scope["tag"]})
				if err != nil || sweep["list_exists"] != false {
					return fmt.Errorf("unconfirmed_fixture_list_remaining")
				}
			}
			cleaned = true
			return os.Remove(recovery)
		},
	}
	w, err := world.NewRealWithSetup(ctx, level, 0, setup)
	add("private_signed_world_and_owned_tagged_fixture", err == nil)
	if err != nil {
		e.Assertions[len(e.Assertions)-1].Detail = "Native world or AppleTruth Reminders access unavailable; no permission changes requested"
		return e
	}
	defer func() {
		w.Close()
		result.Assertions = append(result.Assertions, Assertion{Name: "owned_lists_tagged_items_cleanup_and_sweep", Passed: cleaned})
		if !cleaned || len(w.TruthFailures) > 0 {
			result.Passed = false
		}
	}()
	client := scripted.Client{Endpoint: w.Gateway, Protocol: "rest"}
	find := func(collection, id string) (contracts.Record, bool) {
		var read contracts.ReadResult
		if client.Call(ctx, "records.get", contracts.ReadQuery{Collection: collection, ID: id}, &read) != nil {
			return contracts.Record{}, false
		}
		for _, record := range read.Records {
			if record.ID == id && !record.Deleted {
				return record, true
			}
		}
		return contracts.Record{}, false
	}
	submit := func(op string, args map[string]any, version uint64) (contracts.CommandRecord, error) {
		var command contracts.CommandRecord
		request := map[string]any{"command_id": contracts.NewCommandID(), "args": args, "expected_version": version, "wait_ms": 15000}
		requests = append(requests, map[string]any{"operation": op, "request": request})
		if err := journal(); err != nil {
			return command, err
		}
		err := client.Call(ctx, op, request, &command)
		if err == nil && (command.Disposition != "locally_observed" || len(command.ObservedRecordIDs) != 1) {
			err = fmt.Errorf("command_observation_identity_missing")
		}
		return command, err
	}
	var listA contracts.Record
	ready := w.Await(20*time.Second, func() bool {
		records, err := w.Records(w.Gateway)
		for _, record := range records {
			if record.Collection == "reminders.list" && recordTitle(record) == listName {
				listA = record
				return err == nil
			}
		}
		return false
	})
	add("unique_owned_list_capture", ready == nil)
	if ready != nil {
		return e
	}
	createdList, err := submit("reminders.create_list", map[string]any{"title": destinationName}, 0)
	var listEvidence struct {
		NativeKey string `json:"native_key"`
	}
	json.Unmarshal(createdList.Evidence, &listEvidence)
	add("create_list_native_identity_and_observed_record_id", err == nil && listEvidence.NativeKey != "" && listEvidence.NativeKey != owned[0])
	if err != nil || listEvidence.NativeKey == "" || listEvidence.NativeKey == owned[0] {
		return e
	}
	owned = append(owned, listEvidence.NativeKey)
	owners[listEvidence.NativeKey] = map[string]string{"list": destinationName, "tag": destinationTag}
	if err := journal(); err != nil {
		add("record_created_list_for_recovery", false)
		return e
	}
	listB := createdList.ObservedRecordIDs[0]
	blank, err := submit("reminders.create", map[string]any{"list_id": listA.ID, "title": prefix + " blank fields", "notes": "", "url": "", "due": "", "due_all_day": false, "priority": 0}, 0)
	add("create_blank_optional_fields", err == nil)
	if err != nil {
		return e
	}
	blankTruth, blankErr := w.Truth("observe", map[string]any{"source": "reminders", "list": listName, "list_id": owned[0], "tag": tag, "full_fields": true})
	blankRecords, _ := blankTruth["records"].([]any)
	blankExact := false
	if blankErr == nil && len(blankRecords) == 1 {
		row, _ := blankRecords[0].(map[string]any)
		blankExact = row["notes"] == nil && row["url"] == nil && row["due"] == nil && number(row["priority"], "0")
	}
	add("eventkit_truth_blank_optional_fields_exact", blankExact)
	if !blankExact {
		return e
	}
	blankRecord, foundBlank := find("reminders.reminder", blank.ObservedRecordIDs[0])
	if !foundBlank {
		add("blank_fixture_capture", false)
		return e
	}
	_, err = submit("reminders.delete", map[string]any{"id": blankRecord.ID}, blankRecord.Version)
	add("blank_fixture_deleted", err == nil)
	if err != nil {
		return e
	}
	title := prefix + " web write"
	created, err := submit("reminders.create", map[string]any{"list_id": listA.ID, "title": title, "notes": tag + " notes", "url": "https://example.com/reminder", "due": "2028-01-02", "due_all_day": true, "priority": 1}, 0)
	add("create_observed_identity", err == nil)
	if err != nil {
		return e
	}
	itemID := created.ObservedRecordIDs[0]
	truth := func(list string) ([]any, error) {
		value, err := w.Truth("observe", map[string]any{"source": "reminders", "list": owners[list]["list"], "list_id": list, "tag": owners[list]["tag"], "full_fields": true})
		if err != nil {
			return nil, err
		}
		records, ok := value["records"].([]any)
		if !ok {
			return nil, fmt.Errorf("truth_records_missing")
		}
		return records, nil
	}
	records, err := truth(owned[0])
	exact := false
	if err == nil && len(records) == 1 {
		record, _ := records[0].(map[string]any)
		due, _ := record["due"].(map[string]any)
		exact = record["title"] == title && record["notes"] == tag+" notes" && record["url"] == "https://example.com/reminder" && number(record["priority"], "1") && number(due["year"], "2028") && number(due["month"], "1") && number(due["day"], "2")
	}
	add("eventkit_truth_create_fields_exact", exact)
	if !exact {
		return e
	}
	current, ok := find("reminders.reminder", itemID)
	add("observed_id_locates_created_record", ok)
	if !ok {
		return e
	}
	title = prefix + " edited"
	_, err = submit("reminders.update", map[string]any{"id": itemID, "title": title, "notes": tag + " edited notes", "url": "", "due": "", "due_all_day": false, "priority": 5}, current.Version)
	add("edit_and_clear_date_url", err == nil)
	if err != nil {
		return e
	}
	records, err = truth(owned[0])
	exact = false
	if err == nil && len(records) == 1 {
		record, _ := records[0].(map[string]any)
		exact = record["title"] == title && record["notes"] == tag+" edited notes" && record["due"] == nil && record["url"] == nil && number(record["priority"], "5")
	}
	add("eventkit_truth_edited_fields_and_clears_exact", exact)
	if !exact {
		return e
	}
	for _, op := range []string{"reminders.complete", "reminders.uncomplete"} {
		current, ok = find("reminders.reminder", itemID)
		if !ok {
			add("fresh_version_before_"+op, false)
			return e
		}
		_, err = submit(op, map[string]any{"id": itemID}, current.Version)
		add(op+"_observed", err == nil)
		if err != nil {
			return e
		}
		records, err = truth(owned[0])
		exact = false
		if err == nil && len(records) == 1 {
			record, _ := records[0].(map[string]any)
			exact = record["completed"] == (op == "reminders.complete")
		}
		add(op+"_eventkit_truth", exact)
		if !exact {
			return e
		}
	}
	current, ok = find("reminders.reminder", itemID)
	if !ok {
		add("fresh_version_before_move", false)
		return e
	}
	moved, err := submit("reminders.update", map[string]any{"id": itemID, "list_id": listB, "notes": ""}, current.Version)
	add("move_observed_identity", err == nil)
	if err != nil {
		return e
	}
	itemID = moved.ObservedRecordIDs[0]
	source, sourceErr := truth(owned[0])
	destination, destinationErr := truth(owned[1])
	add("eventkit_truth_move_exact", sourceErr == nil && destinationErr == nil && len(source) == 0 && len(destination) == 1)
	if sourceErr != nil || destinationErr != nil || len(source) != 0 || len(destination) != 1 {
		return e
	}
	destinationRecord, _ := destination[0].(map[string]any)
	add("eventkit_truth_notes_cleared_on_move", destinationRecord["notes"] == nil || destinationRecord["notes"] == "")
	if notes, present := destinationRecord["notes"]; !present {
		e.Metrics["cleared_notes_representation"] = "absent"
	} else if value, ok := notes.(string); ok && value == "" {
		e.Metrics["cleared_notes_representation"] = "empty_string"
	} else {
		e.Metrics["cleared_notes_representation"] = "nonempty_or_invalid"
	}
	current, ok = find("reminders.reminder", itemID)
	if !ok {
		add("fresh_version_before_delete", false)
		return e
	}
	_, err = submit("reminders.delete", map[string]any{"id": itemID}, current.Version)
	add("delete_observed_identity", err == nil)
	if err != nil {
		return e
	}
	records, err = truth(owned[1])
	add("eventkit_truth_delete_exact", err == nil && len(records) == 0)
	e.Passed = true
	for _, assertion := range e.Assertions {
		if !assertion.Passed {
			e.Passed = false
		}
	}
	return e
}
