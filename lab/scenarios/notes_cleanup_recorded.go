package scenarios

import (
	"encoding/json"
	"strings"
)

func notesCleanupRecordedExtras(recover func(string, map[string]any) (map[string]any, error), raw []byte) error {
	var previous Evidence
	if json.Unmarshal(raw, &previous) != nil || previous.Scenario != "NOTE-EXTRAS" || previous.Level != "real" || previous.Passed || !strings.HasPrefix(previous.RunID, "notes-extras_") || strings.ContainsAny(previous.RunID, "[]\r\n") {
		return nil
	}
	created := false
	for _, assertion := range previous.Assertions {
		if assertion.Name == "tagged_fixture_created" && assertion.Passed {
			created = true
		}
	}
	if !created {
		return nil
	}
	tag := "[ab-test " + previous.RunID + "]"
	title := "Bridge Test " + tag
	identity, err := recover("fixture", map[string]any{"source": "notes", "tag": tag, "action": "folder_identity", "title": title, "allow_missing": true})
	if err != nil {
		return err
	}
	id, _ := identity["id"].(string)
	if id == "" {
		return nil
	}
	return notesCleanupFolders(recover, tag, map[string]string{id: title})
}
