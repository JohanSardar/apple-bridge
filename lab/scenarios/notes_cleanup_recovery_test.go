package scenarios

import (
	"encoding/json"
	"testing"
)

func TestNotesCapturedCleanupValidatesAllIdentitiesBeforeMutation(t *testing.T) {
	for _, valid := range []bool{false, true} {
		entries := map[string]cleanupItem{"a": {ID: "a", Tag: "[ab-test own]", Container: "folder"}, "b": {ID: "b", Tag: "[ab-test own]", Container: "folder"}}
		if !valid {
			item := entries["b"]
			item.Tag = "ordinary"
			entries["b"] = item
		}
		raw, _ := json.Marshal(map[string]any{"created": entries})
		calls := 0
		err := notesCleanupCapturedRun(func(verb string, args map[string]any) (map[string]any, error) {
			calls++
			if verb != "fixture" || args["action"] != "cleanup_owned" || args["tag"] != "[ab-test own]" || entries[args["id"].(string)].Container != args["folder_id"] {
				t.Fatal("cleanup widened scope")
			}
			return map[string]any{"status": "verified"}, nil
		}, raw)
		if valid && (err != nil || calls != 2) || !valid && (err == nil || calls != 0) {
			t.Fatalf("err=%v calls=%d valid=%v", err, calls, valid)
		}
	}
}

func TestNotesCapturedFolderCleanupRejectsInvalidManifestBeforeAnyMutation(t *testing.T) {
	for _, valid := range []bool{false, true} {
		title := "Bridge Test [ab-test own]"
		if !valid {
			title = "ordinary folder"
		}
		raw, _ := json.Marshal(map[string]any{"created": map[string]cleanupItem{"note": {ID: "note", Tag: "[ab-test own]", Container: "folder"}}, "folders": []map[string]any{{"id": "folder", "title": title, "tag": "[ab-test own]"}}})
		calls := 0
		err := notesCleanupCapturedRun(func(verb string, args map[string]any) (map[string]any, error) {
			calls++
			if args["tag"] != "[ab-test own]" || verb != "fixture" {
				t.Fatal("cleanup widened the run")
			}
			if args["action"] == "cleanup_created_folders" {
				folders := args["folders"].([]map[string]any)
				if len(folders) != 1 || folders[0]["id"] != "folder" || folders[0]["title"] != "Bridge Test [ab-test own]" {
					t.Fatal("cleanup changed captured folder identity")
				}
			}
			return map[string]any{"status": "verified"}, nil
		}, raw)
		if valid && (err != nil || calls != 2) || !valid && (err == nil || calls != 0) {
			t.Fatalf("err=%v calls=%d valid=%v", err, calls, valid)
		}
	}
}
