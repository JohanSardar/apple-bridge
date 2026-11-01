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
	}
	if json.Unmarshal(raw, &ledger) != nil || len(ledger.Created) == 0 {
		return fmt.Errorf("Notes cleanup needs captured test identities")
	}
	for id, item := range ledger.Created {
		if id == "" || id != item.ID || item.Container == "" || !strings.HasPrefix(item.Tag, "[ab-test ") || !strings.HasSuffix(item.Tag, "]") || strings.ContainsAny(item.Tag, "\r\n") {
			return fmt.Errorf("Notes cleanup identity is not owned by the test run")
		}
	}
	for _, item := range ledger.Created {
		if err := notesCleanupTruth(recover, 30*time.Second, map[string]any{"source": "notes", "action": "cleanup_owned", "tag": item.Tag, "id": item.ID, "folder_id": item.Container}); err != nil {
			return err
		}
	}
	return nil
}
