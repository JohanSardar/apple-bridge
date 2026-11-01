package scenarios

import (
	"sort"
	"time"
)

func notesCleanupFolders(recover func(string, map[string]any) (map[string]any, error), tag string, owned map[string]string) error {
	if len(owned) == 0 {
		return nil
	}
	ids := make([]string, 0, len(owned))
	for id := range owned {
		ids = append(ids, id)
	}
	sort.Strings(ids)
	folders := make([]map[string]any, 0, len(ids))
	for _, id := range ids {
		folders = append(folders, map[string]any{"id": id, "title": owned[id]})
	}
	return notesCleanupTruth(recover, 30*time.Second, map[string]any{"source": "notes", "tag": tag, "action": "cleanup_created_folders", "created_by_run": true, "folders": folders})
}

func notesCleanupTruth(recover func(string, map[string]any) (map[string]any, error), settle time.Duration, request map[string]any) error {
	_, problem := recover("fixture", request)
	if problem == nil {
		return nil
	}
	var ids []string
	if request["action"] == "cleanup_created_folders" {
		folders, _ := request["folders"].([]map[string]any)
		for _, folder := range folders {
			id, _ := folder["id"].(string)
			if id == "" {
				return problem
			}
			ids = append(ids, id)
		}
	} else {
		key := "id"
		if request["action"] == "cleanup_created_folder" {
			key = "folder_id"
		}
		if id, _ := request[key].(string); id != "" {
			ids = []string{id}
		}
	}
	if len(ids) == 0 {
		return problem
	}
	deadline := time.Now().Add(settle)
	for {
		response, err := recover("observe", map[string]any{"source": "notes", "verify_deleted_ids": ids})
		if err == nil && response["status"] == "verified" && response["deletion_verified"] == true {
			return nil
		}
		if !time.Now().Before(deadline) {
			return problem
		}
		time.Sleep(min(250*time.Millisecond, time.Until(deadline)))
	}
}
