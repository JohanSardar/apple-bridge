package scenarios

import (
	"encoding/json"
	"fmt"
	"strings"
	"time"
)

func notesCleanupCapturedRun(recover func(string, map[string]any) (map[string]any, error), raw []byte) error {
	var ledger struct {
		Created map[string]cleanupItem `json:"created"`
		Folders []struct {
			ID    string `json:"id"`
			Title string `json:"title"`
			Tag   string `json:"tag"`
		} `json:"folders"`
	}
	if json.Unmarshal(raw, &ledger) != nil || len(ledger.Created) == 0 && len(ledger.Folders) == 0 {
		return fmt.Errorf("Notes cleanup needs captured test identities")
	}
	for id, item := range ledger.Created {
		if id == "" || id != item.ID || item.Container == "" || !strings.HasPrefix(item.Tag, "[ab-test ") || !strings.HasSuffix(item.Tag, "]") || strings.ContainsAny(item.Tag, "\r\n") {
			return fmt.Errorf("Notes cleanup identity is not owned by the test run")
		}
	}
	groups := map[string]map[string]string{}
	for _, folder := range ledger.Folders {
		base := "Bridge Test " + folder.Tag
		if folder.ID == "" || !strings.HasPrefix(folder.Tag, "[ab-test ") || !strings.HasSuffix(folder.Tag, "]") || strings.ContainsAny(folder.Tag, "\r\n") || (folder.Title != base && !strings.HasPrefix(folder.Title, base+" ")) {
			return fmt.Errorf("Notes cleanup folder is not owned by the test run")
		}
		if groups[folder.Tag] == nil {
			groups[folder.Tag] = map[string]string{}
		}
		if _, exists := groups[folder.Tag][folder.ID]; exists {
			return fmt.Errorf("Notes cleanup folder identity is duplicated")
		}
		groups[folder.Tag][folder.ID] = folder.Title
	}

	for _, item := range ledger.Created {
		if err := notesCleanupTruth(recover, 30*time.Second, map[string]any{"source": "notes", "action": "cleanup_owned", "tag": item.Tag, "id": item.ID, "folder_id": item.Container}); err != nil {
			return err
		}
	}
	for tag, folders := range groups {
		if err := notesCleanupFolders(recover, tag, folders); err != nil {
			return err
		}
	}
	return nil
}
