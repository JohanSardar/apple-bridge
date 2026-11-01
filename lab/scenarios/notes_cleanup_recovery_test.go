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
